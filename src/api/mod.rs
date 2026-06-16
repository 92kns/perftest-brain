pub mod bugzilla;
pub mod perfherder;
pub mod taskcluster;
pub mod treeherder;

use anyhow::{anyhow, Result};

const USER_AGENT: &str = concat!("perftest-brain/", env!("CARGO_PKG_VERSION"));

/// Shared HTTP GET with JSON deserialization, User-Agent, and error handling.
pub(crate) fn get_json<T: serde::de::DeserializeOwned>(url: &str) -> Result<T> {
    let response = ureq::get(url)
        .set("User-Agent", USER_AGENT)
        .set("Accept", "application/json")
        .call()
        .map_err(|e| match e {
            ureq::Error::Status(code, resp) => {
                let body = resp.into_string().unwrap_or_default();
                anyhow!("HTTP {} from {}: {}", code, url, body.trim())
            }
            ureq::Error::Transport(t) => anyhow!("Transport error fetching {}: {}", url, t),
        })?;

    let value: T = response
        .into_json()
        .map_err(|e| anyhow!("Failed to deserialize JSON from {}: {}", url, e))?;

    Ok(value)
}
