use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::tools::{CliTool, Tool};

/// A single searchfox result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchfoxResult {
    pub path: String,
    pub line: u64,
    pub context: String,
}

/// Query searchfox-cli for a test name. [IDX-03]
///
/// Falls back gracefully when searchfox-cli is not installed.
pub fn search_searchfox(query: &str) -> Result<Vec<SearchfoxResult>> {
    let tool = CliTool::new("searchfox-cli");
    if !tool.check_available() {
        return Ok(vec![]);
    }

    let out = tool.run(&["search", query, "--json"])?;
    if out.exit_code != 0 || out.stdout.trim().is_empty() {
        return Ok(vec![]);
    }

    // Try to parse as a JSON array of results
    let parsed: Vec<serde_json::Value> =
        serde_json::from_str(out.stdout.trim()).unwrap_or_default();

    Ok(parsed
        .into_iter()
        .filter_map(|v| {
            Some(SearchfoxResult {
                path: v.get("path")?.as_str()?.to_owned(),
                line: v.get("line").and_then(|l| l.as_u64()).unwrap_or(0),
                context: v
                    .get("context")
                    .and_then(|c| c.as_str())
                    .unwrap_or("")
                    .to_owned(),
            })
        })
        .take(20)
        .collect())
}

/// Search the index; fall back to searchfox-cli if index is empty or returns nothing. [IDX-03]
pub fn search_with_fallback(
    query: &str,
    verbose: bool,
) -> Result<Vec<super::TestEntry>> {
    let local = super::search_tests(query)?;
    if !local.is_empty() {
        return Ok(local);
    }

    if verbose {
        eprintln!("Local index empty for {:?} — falling back to searchfox-cli", query);
    }

    let sfx = search_searchfox(query)?;
    Ok(sfx
        .into_iter()
        .map(|r| super::TestEntry {
            path: r.path,
            name: r.context,
            harness: "unknown".into(),
            platforms: vec![],
            mtime: 0,
        })
        .collect())
}
