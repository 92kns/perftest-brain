pub mod searchfox;

use anyhow::{Context, Result};
use directories::ProjectDirs;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Scoped directories to index — relative to the Firefox checkout root.
/// `testing/performance/` is mozperftest territory (same harness, different path).
pub const INDEX_SCOPE: &[&str] = &[
    "testing/raptor",
    "testing/mozperftest",
    "testing/performance",
    "taskcluster",
];

/// A single indexed test entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestEntry {
    pub path: String,
    pub name: String,
    pub harness: String,
    pub platforms: Vec<String>,
    pub mtime: u64,
}

/// Index statistics.
#[derive(Debug, Serialize, Deserialize)]
pub struct IndexStats {
    pub test_count: usize,
    pub task_count: usize,
    pub last_updated: Option<u64>,
    pub db_path: PathBuf,
}

/// Get the XDG-compliant database path (never ~/). [IDX-02]
pub fn db_path() -> Result<PathBuf> {
    let proj = ProjectDirs::from("org", "mozilla", "perftest-brain")
        .ok_or_else(|| anyhow::anyhow!("Could not determine XDG config directory"))?;
    let data_dir = proj.data_dir();
    std::fs::create_dir_all(data_dir)
        .with_context(|| format!("Could not create data directory: {}", data_dir.display()))?;
    Ok(data_dir.join("index.db"))
}

/// Open (or create) the SQLite index database.
pub fn open_db(db: &Path) -> Result<Connection> {
    let conn = Connection::open(db)
        .with_context(|| format!("Could not open index database at {}", db.display()))?;

    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS tests (
            path TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            harness TEXT NOT NULL,
            platforms TEXT NOT NULL DEFAULT '[]',
            mtime INTEGER NOT NULL DEFAULT 0
        );
        CREATE TABLE IF NOT EXISTS tasks (
            path TEXT PRIMARY KEY,
            task_id_pattern TEXT NOT NULL DEFAULT '',
            mtime INTEGER NOT NULL DEFAULT 0
        );
        CREATE TABLE IF NOT EXISTS meta (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );",
    )?;

    Ok(conn)
}

/// Build or update the local index by walking the Firefox checkout. [IDX-01]
///
/// Only walks `INDEX_SCOPE` directories — never the full checkout. [IDX-02]
pub fn update_index(checkout_root: &Path, verbose: bool) -> Result<IndexStats> {
    let db_path = db_path()?;
    let conn = open_db(&db_path)?;

    let mut test_count = 0usize;
    let mut task_count = 0usize;

    for scope in INDEX_SCOPE {
        let scope_dir = checkout_root.join(scope);
        if !scope_dir.exists() {
            if verbose {
                eprintln!("Skipping (not found): {}", scope_dir.display());
            }
            continue;
        }

        if verbose {
            eprintln!("Indexing: {}", scope_dir.display());
        }

        let walker = ignore::WalkBuilder::new(&scope_dir)
            .hidden(false)
            .git_ignore(true)
            .build();

        for entry in walker.flatten() {
            let path = entry.path();
            let rel = path
                .strip_prefix(checkout_root)
                .unwrap_or(path)
                .to_string_lossy()
                .into_owned();

            let mtime = path
                .metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
                .unwrap_or(0);

            let file_name = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_lowercase();

            // Index test manifests and test files
            if is_test_file(&file_name) {
                let harness = infer_harness(scope);
                let name = extract_test_name(path, checkout_root);

                conn.execute(
                    "INSERT OR REPLACE INTO tests (path, name, harness, platforms, mtime)
                     VALUES (?1, ?2, ?3, '[]', ?4)",
                    params![rel, name, harness, mtime as i64],
                )?;
                test_count += 1;
            }

            // Index taskcluster task YAML files
            if scope.starts_with("taskcluster") && is_task_file(&file_name) {
                conn.execute(
                    "INSERT OR REPLACE INTO tasks (path, task_id_pattern, mtime)
                     VALUES (?1, '', ?2)",
                    params![rel, mtime as i64],
                )?;
                task_count += 1;
            }
        }
    }

    // Record last-updated timestamp
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    conn.execute(
        "INSERT OR REPLACE INTO meta (key, value) VALUES ('last_updated', ?1)",
        params![now.to_string()],
    )?;

    Ok(IndexStats {
        test_count,
        task_count,
        last_updated: Some(now),
        db_path,
    })
}

/// Look up tests by name in the local index. [IDX-03 fallback is in caller]
pub fn search_tests(query: &str) -> Result<Vec<TestEntry>> {
    let db_path = db_path()?;
    if !db_path.exists() {
        return Ok(vec![]);
    }

    let conn = open_db(&db_path)?;
    let pattern = format!("%{}%", query.to_lowercase());

    let mut stmt = conn.prepare(
        "SELECT path, name, harness, platforms, mtime FROM tests
         WHERE lower(name) LIKE ?1 OR lower(path) LIKE ?1
         LIMIT 50",
    )?;

    let entries = stmt
        .query_map(params![pattern], |row| {
            let platforms_json: String = row.get(3)?;
            Ok(TestEntry {
                path: row.get(0)?,
                name: row.get(1)?,
                harness: row.get(2)?,
                platforms: serde_json::from_str(&platforms_json).unwrap_or_default(),
                mtime: row.get::<_, i64>(4)? as u64,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;

    Ok(entries)
}

/// Get index statistics.
pub fn index_stats() -> Result<IndexStats> {
    let db_path = db_path()?;
    if !db_path.exists() {
        return Ok(IndexStats {
            test_count: 0,
            task_count: 0,
            last_updated: None,
            db_path,
        });
    }

    let conn = open_db(&db_path)?;
    let test_count: usize = conn.query_row("SELECT COUNT(*) FROM tests", [], |r| r.get(0))?;
    let task_count: usize = conn.query_row("SELECT COUNT(*) FROM tasks", [], |r| r.get(0))?;
    let last_updated: Option<u64> = conn
        .query_row(
            "SELECT value FROM meta WHERE key = 'last_updated'",
            [],
            |r| r.get::<_, String>(0),
        )
        .ok()
        .and_then(|s| s.parse().ok());

    Ok(IndexStats {
        test_count,
        task_count,
        last_updated,
        db_path,
    })
}

fn is_test_file(name: &str) -> bool {
    name.ends_with(".toml")
        || name.ends_with(".ini")
        || name.ends_with(".js")
        || name.ends_with(".py")
}

fn is_task_file(name: &str) -> bool {
    name.ends_with(".yml") || name.ends_with(".yaml")
}

fn infer_harness(scope: &str) -> &'static str {
    if scope.starts_with("testing/raptor") {
        "raptor"
    } else if scope.starts_with("testing/mozperftest") || scope.starts_with("testing/performance") {
        "mozperftest"
    } else if scope.starts_with("taskcluster") {
        "taskcluster"
    } else {
        "unknown"
    }
}

fn extract_test_name(path: &Path, checkout_root: &Path) -> String {
    path.strip_prefix(checkout_root)
        .unwrap_or(path)
        .to_string_lossy()
        .into_owned()
}
