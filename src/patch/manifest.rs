use anyhow::{bail, Context, Result};
use std::path::Path;

/// A fix to apply to a manifest/INI/TOML file.
#[derive(Debug, Clone)]
pub struct ManifestFix {
    pub kind: FixKind,
    pub target_file: String,
    pub description: String,
}

/// The type of manifest fix to apply.
#[derive(Debug, Clone)]
pub enum FixKind {
    /// Add `requestLongerTimeout(N)` to the test entry.
    RequestLongerTimeout { multiplier: u32 },
    /// Add a `skip-if` condition to the test entry.
    SkipIf { condition: String, comment: String },
}

impl ManifestFix {
    pub fn longer_timeout(target_file: impl Into<String>, multiplier: u32) -> Self {
        Self {
            kind: FixKind::RequestLongerTimeout { multiplier },
            target_file: target_file.into(),
            description: format!("Add requestLongerTimeout({multiplier})"),
        }
    }

    pub fn skip_if(
        target_file: impl Into<String>,
        condition: impl Into<String>,
        comment: impl Into<String>,
    ) -> Self {
        let condition = condition.into();
        let comment = comment.into();
        Self {
            description: format!("Add skip-if: {condition}"),
            kind: FixKind::SkipIf { condition, comment },
            target_file: target_file.into(),
        }
    }
}

/// Apply a fix to the target manifest file, reading on-disk content at edit
/// time (never from a stale cache). Writes atomically via tempfile. [PATCH-04]
pub fn apply_fix(fix: &ManifestFix, checkout_root: &Path) -> Result<String> {
    let target = checkout_root.join(&fix.target_file);
    if !target.exists() {
        bail!("Target file not found: {}", target.display());
    }

    let original = std::fs::read_to_string(&target)
        .with_context(|| format!("Could not read {}", target.display()))?;

    let patched = patch_content(&original, fix)?;

    if patched == original {
        return Ok(format!(
            "No change needed — fix already applied to {}",
            fix.target_file
        ));
    }

    // Atomic write: write to a tempfile in the same directory, then rename. [PATCH-04]
    let dir = target.parent().unwrap_or(Path::new("."));
    let tmp = tempfile::NamedTempFile::new_in(dir)
        .with_context(|| format!("Could not create tempfile in {}", dir.display()))?;

    std::io::Write::write_all(&mut tmp.as_file(), patched.as_bytes())
        .with_context(|| "Could not write patched content to tempfile")?;

    tmp.persist(&target)
        .with_context(|| format!("Could not atomically replace {}", target.display()))?;

    Ok(format!("Patched: {}", fix.target_file))
}

fn patch_content(original: &str, fix: &ManifestFix) -> Result<String> {
    match &fix.kind {
        FixKind::RequestLongerTimeout { multiplier } => {
            insert_or_replace_directive(original, "requestLongerTimeout", &multiplier.to_string())
        }
        FixKind::SkipIf { condition, comment } => insert_skip_if(original, condition, comment),
    }
}

/// Insert or update a `key = value` directive in an INI-style manifest.
///
/// If the directive already exists, replaces its value. Otherwise inserts it
/// near the top of the first `[test]`-like section, or at the file start.
fn insert_or_replace_directive(content: &str, key: &str, value: &str) -> Result<String> {
    let new_line = format!("{key} = {value}");
    let key_lower = key.to_lowercase();

    // Replace existing directive
    let mut found = false;
    let mut lines: Vec<&str> = content.lines().collect();
    for line in lines.iter_mut() {
        let trimmed = line.trim_start();
        if trimmed.to_lowercase().starts_with(&key_lower)
            && trimmed[key_lower.len()..].trim_start().starts_with('=')
        {
            *line = Box::leak(new_line.clone().into_boxed_str());
            found = true;
            break;
        }
    }

    if found {
        return Ok(lines.join("\n") + if content.ends_with('\n') { "\n" } else { "" });
    }

    // Insert after first section header, or at top if no section
    let mut result = String::with_capacity(content.len() + new_line.len() + 1);
    let mut inserted = false;
    for line in content.lines() {
        result.push_str(line);
        result.push('\n');
        if !inserted && line.trim_start().starts_with('[') {
            result.push_str(&new_line);
            result.push('\n');
            inserted = true;
        }
    }
    if !inserted {
        // No section header — prepend
        result = format!("{new_line}\n{result}");
    }

    Ok(result)
}

/// Insert a `skip-if` condition into an INI manifest.
fn insert_skip_if(content: &str, condition: &str, comment: &str) -> Result<String> {
    let value = if comment.is_empty() {
        condition.to_owned()
    } else {
        format!("{condition}  # {comment}")
    };
    insert_or_replace_directive(content, "skip-if", &value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn inserts_longer_timeout_directive() {
        let content = "[test_raptor_speedometer]\nurl = https://example.com\n";
        let result = insert_or_replace_directive(content, "requestLongerTimeout", "2").unwrap();
        assert!(
            result.contains("requestLongerTimeout = 2"),
            "result: {result}"
        );
    }

    #[test]
    fn replaces_existing_directive() {
        let content = "[test]\nrequestLongerTimeout = 1\nurl = https://x\n";
        let result = insert_or_replace_directive(content, "requestLongerTimeout", "3").unwrap();
        assert!(result.contains("requestLongerTimeout = 3"));
        assert!(!result.contains("requestLongerTimeout = 1"));
    }

    #[test]
    fn atomic_write_produces_correct_file() {
        let tmp = tempdir().unwrap();
        let target = tmp.path().join("test.ini");
        fs::write(&target, "[test]\nurl = https://x\n").unwrap();

        let fix = ManifestFix::longer_timeout("test.ini", 2);
        let result = apply_fix(&fix, tmp.path()).unwrap();
        assert!(result.contains("Patched"), "result: {result}");

        let content = fs::read_to_string(&target).unwrap();
        assert!(
            content.contains("requestLongerTimeout = 2"),
            "content: {content}"
        );
    }

    #[test]
    fn idempotent_when_already_applied() {
        let tmp = tempdir().unwrap();
        let target = tmp.path().join("test.ini");
        fs::write(
            &target,
            "[test]\nrequestLongerTimeout = 2\nurl = https://x\n",
        )
        .unwrap();

        let fix = ManifestFix::longer_timeout("test.ini", 2);
        let result = apply_fix(&fix, tmp.path()).unwrap();
        assert!(result.contains("No change needed"), "result: {result}");
    }
}
