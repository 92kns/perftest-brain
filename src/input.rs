use anyhow::{anyhow, bail, Result};
use url::Url;

use crate::types::{InputSpec, Push};

const PERFCOMPARE_HOSTS: &[&str] = &[
    "perf.compare.firefox.com",
    "perfcompare.surge.sh",
];
const TREEHERDER_HOSTS: &[&str] = &["treeherder.mozilla.org"];
const BUGZILLA_HOSTS: &[&str] = &["bugzilla.mozilla.org"];

/// Parse any supported input string into an `InputSpec`.
///
/// Accepted formats:
/// - Plain integer → alert ID
/// - 12-40 hex chars → push revision (assumes autoland)
/// - Bugzilla URL → bug ID
/// - Treeherder perfherder/alerts URL → alert ID
/// - Treeherder jobs URL → push
/// - PerfCompare URL → revision pair or Lando pair
pub fn parse_input(raw: &str) -> Result<InputSpec> {
    let raw = raw.trim();

    // Plain integer → alert ID
    if raw.chars().all(|c| c.is_ascii_digit()) && !raw.is_empty() {
        let id: u64 = raw.parse()?;
        return Ok(InputSpec::Alert { alert_id: id });
    }

    // 12–40 hex chars → push revision on autoland
    if raw.len() >= 12
        && raw.len() <= 40
        && raw.chars().all(|c| c.is_ascii_hexdigit())
    {
        return Ok(InputSpec::Push {
            push: Push::autoland(raw),
        });
    }

    // Everything else must be a URL
    let url = Url::parse(raw)
        .map_err(|_| anyhow!("Cannot parse input: {:?} — expected a URL, alert ID, or revision hash", raw))?;

    let host = url.host_str().unwrap_or("");

    if BUGZILLA_HOSTS.iter().any(|h| host == *h) {
        return parse_bugzilla_url(&url, raw);
    }

    if TREEHERDER_HOSTS.iter().any(|h| host == *h) {
        return parse_treeherder_url(&url, raw);
    }

    if is_perfcompare_host(host) {
        return parse_perfcompare_url(&url, raw);
    }

    bail!("Unrecognised URL host {:?} — needs manual triage", host)
}

fn is_perfcompare_host(host: &str) -> bool {
    PERFCOMPARE_HOSTS
        .iter()
        .any(|h| host == *h || host.ends_with(&format!(".{h}")))
        || host.contains("perfcompare")
}

fn parse_bugzilla_url(url: &Url, raw: &str) -> Result<InputSpec> {
    let id = url
        .query_pairs()
        .find(|(k, _)| k == "id")
        .and_then(|(_, v)| v.parse::<u64>().ok())
        .ok_or_else(|| anyhow!("Could not extract bug ID from Bugzilla URL: {}", raw))?;
    Ok(InputSpec::Bug { bug_id: id })
}

fn parse_treeherder_url(url: &Url, raw: &str) -> Result<InputSpec> {
    let path = url.path();
    let params: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();

    if path.contains("/perfherder/alerts") {
        let id = params
            .get("id")
            .and_then(|v| v.parse::<u64>().ok())
            .ok_or_else(|| anyhow!("Could not extract alert ID from Treeherder URL: {}", raw))?;
        return Ok(InputSpec::Alert { alert_id: id });
    }

    if let Some(rev) = params.get("revision") {
        let repo = params
            .get("repo")
            .cloned()
            .unwrap_or_else(|| "autoland".into());
        return Ok(InputSpec::Push {
            push: Push::new(rev, repo),
        });
    }

    bail!("Could not parse Treeherder URL: {}", raw)
}

fn parse_perfcompare_url(url: &Url, _raw: &str) -> Result<InputSpec> {
    let params: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();

    // Lando-based compare
    if let (Some(base_lando), Some(new_lando)) =
        (params.get("baseLando"), params.get("newLando"))
    {
        return Ok(InputSpec::Lando {
            base_lando_id: base_lando.clone(),
            new_lando_id: new_lando.clone(),
            base_repo: params
                .get("baseRepo")
                .cloned()
                .unwrap_or_else(|| "autoland".into()),
            new_repo: params
                .get("newRepo")
                .cloned()
                .unwrap_or_else(|| "autoland".into()),
        });
    }

    // Revision-based compare
    if let (Some(base_rev), Some(new_rev)) = (params.get("baseRev"), params.get("newRev")) {
        let base_repo = params
            .get("baseRepo")
            .cloned()
            .unwrap_or_else(|| "autoland".into());
        let new_repo = params
            .get("newRepo")
            .cloned()
            .unwrap_or_else(|| "autoland".into());
        return Ok(InputSpec::PerfCompare {
            base: Push::new(base_rev, base_repo),
            new: Push::new(new_rev, new_repo),
        });
    }

    bail!("Could not parse PerfCompare URL — expected baseLando/newLando or baseRev/newRev params")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_integer_is_alert() {
        match parse_input("44793").unwrap() {
            InputSpec::Alert { alert_id: 44793 } => {}
            other => panic!("expected Alert, got {other:?}"),
        }
    }

    #[test]
    fn hex_revision_is_push() {
        match parse_input("4bfc5585ab5d").unwrap() {
            InputSpec::Push { push } => assert_eq!(push.repo, "autoland"),
            other => panic!("expected Push, got {other:?}"),
        }
    }

    #[test]
    fn treeherder_perfherder_alert_url() {
        match parse_input(
            "https://treeherder.mozilla.org/perfherder/alerts?id=44793",
        )
        .unwrap()
        {
            InputSpec::Alert { alert_id: 44793 } => {}
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn treeherder_push_url() {
        match parse_input(
            "https://treeherder.mozilla.org/jobs?repo=autoland&revision=abc123def456",
        )
        .unwrap()
        {
            InputSpec::Push { push } => {
                assert_eq!(push.revision, "abc123def456");
                assert_eq!(push.repo, "autoland");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn bugzilla_url() {
        match parse_input(
            "https://bugzilla.mozilla.org/show_bug.cgi?id=2042450",
        )
        .unwrap()
        {
            InputSpec::Bug { bug_id: 2042450 } => {}
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn perfcompare_rev_url() {
        match parse_input(
            "https://perf.compare.firefox.com/?baseRev=aaa&newRev=bbb&baseRepo=autoland&newRepo=autoland",
        )
        .unwrap()
        {
            InputSpec::PerfCompare { base, new } => {
                assert_eq!(base.revision, "aaa");
                assert_eq!(new.revision, "bbb");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn unknown_host_needs_manual_triage() {
        let err = parse_input("https://example.com/foo").unwrap_err();
        assert!(err.to_string().contains("manual triage"));
    }

    #[test]
    fn garbage_input_errors() {
        assert!(parse_input("not-a-url-or-id").is_err());
    }
}
