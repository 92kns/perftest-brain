use anyhow::{anyhow, Result};
use serde::Deserialize;
use std::collections::BTreeMap;

use crate::types::Push;

const HG_BASE: &str = "https://hg.mozilla.org";

fn hg_path(repo: &str) -> String {
    match repo {
        "autoland" => "integration/autoland".into(),
        "mozilla-central" => "mozilla-central".into(),
        "mozilla-beta" => "releases/mozilla-beta".into(),
        "mozilla-release" => "releases/mozilla-release".into(),
        "try" => "try".into(),
        other => format!("integration/{other}"),
    }
}

/// A single commit in the regression window.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Commit {
    pub node: String,
    pub short_node: String,
    pub author: String,
    pub desc: String,
    pub short_desc: String,
    pub files: Vec<String>,
    pub bug_id: Option<String>,
    pub is_noise: bool,
}

#[derive(Deserialize)]
struct HgPush {
    changesets: Vec<HgChangeset>,
}

#[derive(Deserialize)]
struct HgChangeset {
    node: String,
    author: String,
    desc: String,
    #[serde(default)]
    files: Vec<String>,
}

static NOISE_KEYWORDS: &[&str] = &[
    "DONTBUILD",
    "l10n-bump",
    "l10n changesets",
    "version bump",
    "merge mozilla",
    "merge autoland",
    "merge beta",
    "merge release",
    "merge central",
];

fn is_noise(desc: &str) -> bool {
    NOISE_KEYWORDS
        .iter()
        .any(|kw| desc.to_lowercase().contains(&kw.to_lowercase()))
}

fn extract_bug_id(desc: &str) -> Option<String> {
    // Match "Bug 12345" or "bug 12345"
    let lower = desc.to_lowercase();
    let idx = lower.find("bug ")?;
    let after = &desc[idx + 4..];
    let end = after
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(after.len());
    let num = &after[..end];
    if num.is_empty() {
        None
    } else {
        Some(num.to_owned())
    }
}

fn parse_changeset(cs: HgChangeset) -> Commit {
    let short_node = cs.node.chars().take(12).collect();
    let short_desc = cs
        .desc
        .lines()
        .next()
        .unwrap_or("")
        .chars()
        .take(100)
        .collect();
    let noise = is_noise(&cs.desc);
    let bug_id = extract_bug_id(&cs.desc);
    Commit {
        short_node,
        node: cs.node,
        author: cs.author,
        bug_id,
        is_noise: noise,
        short_desc,
        desc: cs.desc,
        files: cs.files,
    }
}

/// Fetch all commits between `base` (exclusive) and `new_push` (inclusive).
pub fn fetch_commit_window(base: &Push, new_push: &Push) -> Result<Vec<Commit>> {
    let path = hg_path(&base.repo);
    let url = format!(
        "{HG_BASE}/{path}/json-pushes\
         ?fromchange={}&tochange={}&full=1",
        base.revision, new_push.revision
    );

    let body = ureq::get(&url)
        .set(
            "User-Agent",
            concat!("perftest-brain/", env!("CARGO_PKG_VERSION")),
        )
        .call()
        .map_err(|e| anyhow!("Mercurial pushlog error for {}: {}", base.repo, e))?
        .into_string()?;

    let data: BTreeMap<String, HgPush> =
        serde_json::from_str(&body).map_err(|e| anyhow!("Could not parse pushlog JSON: {}", e))?;

    // BTreeMap gives sorted push IDs = chronological order
    let commits = data
        .into_values()
        .flat_map(|p| p.changesets)
        .map(parse_changeset)
        .collect();

    Ok(commits)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noise_detection() {
        assert!(is_noise("l10n-bump: Updated locales"));
        assert!(is_noise("DONTBUILD - skip CI"));
        assert!(is_noise("version bump for Firefox 123"));
        assert!(!is_noise(
            "Bug 1234567 - Fix performance regression in SpiderMonkey"
        ));
    }

    #[test]
    fn bug_id_extraction() {
        assert_eq!(
            extract_bug_id("Bug 1234567 - fix thing"),
            Some("1234567".into())
        );
        assert_eq!(extract_bug_id("bug 42 landed"), Some("42".into()));
        assert_eq!(extract_bug_id("no bug here"), None);
    }
}
