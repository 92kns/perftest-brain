use anyhow::{bail, Result};

/// Fetch failure logs for a Treeherder job URL via `treeherder-cli`.
///
/// `treeherder-cli` ships with the Firefox repo and is available in any
/// mozilla-central checkout. This mirrors the pattern from car-mechanic-cli.
pub fn fetch_failure_logs(treeherder_url: &str) -> Result<String> {
    eprintln!("Fetching failure logs via treeherder-cli...");

    let output = std::process::Command::new("treeherder-cli")
        .args([treeherder_url, "--fetch-logs", "--match-filter", "failure"])
        .output()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                anyhow::anyhow!(
                    "treeherder-cli not found on PATH.\n\
                     It ships with the Firefox repo — make sure you are running\n\
                     perftest-brain from inside a mozilla-central checkout."
                )
            } else {
                anyhow::anyhow!("running treeherder-cli: {}", e)
            }
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("treeherder-cli failed:\n{}", stderr);
    }

    let text = String::from_utf8_lossy(&output.stdout).into_owned();
    if text.trim().is_empty() {
        bail!(
            "treeherder-cli returned no output. The job may not have failed yet,\n\
             or no failing performance jobs matched. Retrigger and try again."
        );
    }

    Ok(text)
}

/// Build a Treeherder job URL from a push revision and repo.
pub fn treeherder_url(revision: &str, repo: &str) -> String {
    format!(
        "https://treeherder.mozilla.org/jobs?repo={}&revision={}",
        repo, revision
    )
}
