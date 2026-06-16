use anyhow::Result;
use serde::Deserialize;

use crate::api::get_json;
use crate::types::{AlertResult, AlertSummary, Push};

const TREEHERDER_BASE: &str = "https://treeherder.mozilla.org/api";

fn framework_name(id: u64) -> String {
    match id {
        1 => "talos",
        2 => "build_metrics",
        4 => "awsy",
        6 => "platform_microbench",
        11 => "js-bench",
        12 => "devtools",
        13 => "browsertime",
        15 => "mozperftest",
        16 => "fxrecord",
        17 => "telemetry",
        18 => "mozharness",
        _ => "unknown",
    }
    .into()
}

fn alert_status_name(code: u64) -> String {
    match code {
        0 => "untriaged",
        1 => "downstream",
        2 => "reassigned",
        3 => "invalid",
        4 => "improvement",
        5 => "investigating",
        6 => "fixed",
        7 => "wontfix",
        _ => "unknown",
    }
    .into()
}

// ── Raw Perfherder response types ────────────────────────────────────────────

#[derive(Deserialize)]
struct PhSeriesSignature {
    suite: String,
    test: String,
    machine_platform: String,
    #[serde(default)]
    suite_public_name: Option<String>,
    #[serde(default)]
    test_public_name: Option<String>,
}

#[derive(Deserialize)]
struct PhAlert {
    status: u64,
    is_regression: bool,
    #[serde(default)]
    prev_value: f64,
    #[serde(default)]
    new_value: f64,
    #[serde(default)]
    amount_pct: f64,
    series_signature: PhSeriesSignature,
    #[serde(default)]
    profile_url: Option<String>,
    #[serde(default)]
    prev_profile_url: Option<String>,
}

#[derive(Deserialize)]
struct PhAlertSummary {
    id: u64,
    status: u64,
    framework: u64,
    #[serde(default)]
    repository: Option<String>,
    push_id: u64,
    prev_push_id: u64,
    #[serde(default)]
    alerts: Vec<PhAlert>,
    #[serde(default)]
    prev_push_revision: Option<String>,
    #[serde(default)]
    revision: Option<String>,
}

#[derive(Deserialize)]
struct PhAlertSummaryList {
    results: Vec<PhAlertSummary>,
}

// ── Public API ────────────────────────────────────────────────────────────────

pub fn fetch_alert_summary(alert_id: u64) -> Result<AlertSummary> {
    let raw: PhAlertSummary = get_json(&format!(
        "{TREEHERDER_BASE}/performance/alertsummary/{alert_id}/"
    ))?;
    Ok(convert_summary(raw))
}

pub fn fetch_alert_summaries_for_bug(bug_id: u64) -> Result<Vec<AlertSummary>> {
    let data: PhAlertSummaryList = get_json(&format!(
        "{TREEHERDER_BASE}/performance/alertsummary/?bug_id={bug_id}"
    ))?;
    Ok(data.results.into_iter().map(convert_summary).collect())
}

fn convert_summary(raw: PhAlertSummary) -> AlertSummary {
    let repo = raw.repository.unwrap_or_else(|| "mozilla-central".into());
    let base_push = Push::new(
        raw.prev_push_revision.unwrap_or_default(),
        repo.clone(),
    );
    let new_push = Push::new(raw.revision.unwrap_or_default(), repo.clone());

    let all_results: Vec<AlertResult> = raw
        .alerts
        .iter()
        .filter(|a| a.status != 3) // filter out "invalid"
        .map(|a| AlertResult {
            test: a
                .series_signature
                .test_public_name
                .clone()
                .unwrap_or_else(|| a.series_signature.test.clone()),
            platform: a.series_signature.machine_platform.clone(),
            suite: a
                .series_signature
                .suite_public_name
                .clone()
                .unwrap_or_else(|| a.series_signature.suite.clone()),
            prev_value: a.prev_value,
            new_value: a.new_value,
            delta_percent: if a.is_regression {
                a.amount_pct
            } else {
                -a.amount_pct
            },
            is_regression: a.is_regression,
            has_profile: a.profile_url.is_some() || a.prev_profile_url.is_some(),
            profile_url: a.profile_url.clone(),
            base_push: base_push.clone(),
            new_push: new_push.clone(),
        })
        .collect();

    AlertSummary {
        id: raw.id,
        status: alert_status_name(raw.status),
        framework: framework_name(raw.framework),
        repository: repo,
        push_id: raw.push_id,
        prev_push_id: raw.prev_push_id,
        regressions: all_results
            .iter()
            .filter(|r| r.is_regression)
            .cloned()
            .collect(),
        improvements: all_results
            .iter()
            .filter(|r| !r.is_regression)
            .cloned()
            .collect(),
    }
}
