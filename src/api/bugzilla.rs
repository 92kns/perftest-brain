use anyhow::Result;
use serde::Deserialize;

use crate::api::get_json;

const BUGZILLA_BASE: &str = "https://bugzilla.mozilla.org/rest";

#[derive(Debug, Clone, serde::Serialize, Deserialize)]
pub struct Bug {
    pub id: u64,
    pub summary: String,
    pub status: String,
    pub resolution: String,
    #[serde(default)]
    pub whiteboard: String,
    #[serde(default)]
    pub keywords: Vec<String>,
}

#[derive(Deserialize)]
struct BugResponse {
    bugs: Vec<Bug>,
}

pub fn fetch_bug(bug_id: u64) -> Result<Bug> {
    let url = format!("{BUGZILLA_BASE}/bug/{bug_id}?include_fields=id,summary,status,resolution,whiteboard,keywords");
    let resp: BugResponse = get_json(&url)?;
    resp.bugs
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("Bug {} not found in Bugzilla", bug_id))
}

/// Search for existing intermittent bugs for a given test name.
pub fn search_intermittent_bugs(test_name: &str) -> Result<Vec<Bug>> {
    let encoded = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("summary", test_name)
        .append_pair("product", "Testing")
        .append_pair("component", "General")
        .append_pair(
            "include_fields",
            "id,summary,status,resolution,whiteboard,keywords",
        )
        .finish();
    let url = format!("{BUGZILLA_BASE}/bug?{encoded}");
    let resp: BugResponse = get_json(&url)?;
    Ok(resp.bugs)
}
