use anyhow::Result;
use serde::Deserialize;

use crate::api::get_json;
use crate::types::ArtifactRef;

const TC_BASE: &str = "https://firefox-ci-tc.services.mozilla.com/api/queue/v1";

#[derive(Deserialize)]
struct ArtifactsResponse {
    #[serde(default)]
    artifacts: Vec<ArtifactEntry>,
}

#[derive(Deserialize)]
struct ArtifactEntry {
    name: String,
}

pub fn list_artifacts_for_task(task_id: &str) -> Result<Vec<ArtifactRef>> {
    let url = format!("{TC_BASE}/task/{task_id}/artifacts");
    let resp: ArtifactsResponse = get_json(&url)?;

    let refs = resp
        .artifacts
        .into_iter()
        .map(|a| ArtifactRef {
            url: format!("{TC_BASE}/task/{task_id}/artifacts/{}", a.name),
            task_id: task_id.to_owned(),
            name: a.name,
        })
        .collect();

    Ok(refs)
}

