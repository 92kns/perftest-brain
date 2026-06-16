use anyhow::{anyhow, Result};
use serde::Deserialize;
use serde_json::Value;

use crate::api::get_json;
use crate::types::Job;

const TREEHERDER_BASE: &str = "https://treeherder.mozilla.org/api";

/// Column-indexed jobs response from the Treeherder API.
/// The API returns `job_property_names` as a header array and `results`
/// as arrays of values — indices must be resolved at runtime, never hardcoded.
#[derive(Deserialize)]
struct JobsResponse {
    job_property_names: Vec<String>,
    results: Vec<Vec<Value>>,
}

pub fn fetch_jobs_for_push(push_id: u64, repo: &str) -> Result<Vec<Job>> {
    let url = format!(
        "{TREEHERDER_BASE}/jobs/?push_id={push_id}&repo={repo}&count=2000"
    );
    let resp: JobsResponse = get_json(&url)?;

    let fields = &resp.job_property_names;
    let idx = |name: &str| {
        fields
            .iter()
            .position(|f| f == name)
            .ok_or_else(|| anyhow!("Treeherder jobs response missing field {:?}", name))
    };

    let i_id = idx("id")?;
    let i_name = idx("job_type_name")?;
    let i_platform = idx("platform")?;
    let i_result = idx("result")?;
    let i_task_id = idx("task_id")?;

    let jobs = resp
        .results
        .into_iter()
        .map(|row| {
            Ok(Job {
                id: row
                    .get(i_id)
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0),
                job_type_name: row
                    .get(i_name)
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_owned(),
                platform: row
                    .get(i_platform)
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_owned(),
                result: row
                    .get(i_result)
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_owned(),
                task_id: row
                    .get(i_task_id)
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_owned(),
            })
        })
        .collect::<Result<Vec<_>>>()?;

    Ok(jobs)
}
