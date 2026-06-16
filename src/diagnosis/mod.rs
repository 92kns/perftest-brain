pub mod patterns;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::api;
use crate::logs;
use crate::types::{AlertSummary, InputSpec};

/// Minimum number of runs required before diagnosis.
const MIN_RUNS_FOR_DIAGNOSIS: usize = 3;

/// A structured diagnosis result.
#[derive(Debug, Serialize, Deserialize)]
pub struct Diagnosis {
    pub input_summary: String,
    pub signal_type: SignalType,
    pub failure_rate: Option<FailureRate>,
    pub findings: Vec<Finding>,
    pub existing_bugs: Vec<ExistingBug>,
    pub next_steps: Vec<String>,
    pub confidence: Confidence,
    pub noise_context: Option<String>,
}

/// Whether this signal looks like an intermittent or a sustained regression.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SignalType {
    /// High-variance, frequency-based — appears and disappears randomly.
    Intermittent,
    /// Sustained shift — Perfherder's t-test flagged this.
    SustainedRegression,
    /// Not enough data to classify.
    Inconclusive,
}

/// Failure rate from retrigger history.
#[derive(Debug, Serialize, Deserialize)]
pub struct FailureRate {
    pub failures: usize,
    pub total_runs: usize,
    pub rate_percent: f64,
}

/// A single diagnosed finding.
#[derive(Debug, Serialize, Deserialize)]
pub struct Finding {
    pub category: String,
    pub description: String,
    pub root_cause: String,
    pub next_step: String,
    pub matched_pattern: Option<String>,
}

/// An existing Bugzilla bug for this failure.
#[derive(Debug, Serialize, Deserialize)]
pub struct ExistingBug {
    pub id: u64,
    pub summary: String,
    pub status: String,
    pub resolution: String,
}

/// How confident we are in the diagnosis.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Confidence {
    High,
    Medium,
    Low,
    Insufficient,
}

/// Diagnose a resolved input spec.
///
/// Returns `Err` for genuine errors, and a `Diagnosis` with `confidence: Insufficient`
/// when there is not enough data (caller should surface the retrigger recommendation).
pub fn diagnose(spec: &InputSpec, verbose: bool) -> Result<Diagnosis> {
    match spec {
        InputSpec::Alert { alert_id } => diagnose_alert(*alert_id, None, verbose),
        InputSpec::Push { push } => {
            // Fetch logs directly from Treeherder via treeherder-cli
            let url = logs::treeherder_url(&push.revision, &push.repo);
            let log_text = logs::fetch_failure_logs(&url)?;
            let input_summary = format!("Push {} ({})", push.revision, push.repo);
            Ok(diagnose_from_log(&log_text, &input_summary))
        }
        InputSpec::Bug { bug_id } => {
            let bug = api::bugzilla::fetch_bug(*bug_id)?;
            let alerts =
                api::perfherder::fetch_alert_summaries_for_bug(*bug_id).unwrap_or_default();
            if let Some(first) = alerts.first() {
                diagnose_alert(
                    first.id,
                    Some(format!("Bug {}: {}", bug.id, bug.summary)),
                    verbose,
                )
            } else {
                Ok(insufficient_diagnosis(format!(
                    "Bug {}: {} — no linked Perfherder alerts found. Pass an alert ID directly.",
                    bug.id, bug.summary
                )))
            }
        }
        InputSpec::PerfCompare { base, new } => {
            let url = logs::treeherder_url(&new.revision, &new.repo);
            let log_text = logs::fetch_failure_logs(&url)?;
            let input_summary = format!("PerfCompare base={} new={}", base.revision, new.revision);
            Ok(diagnose_from_log(&log_text, &input_summary))
        }
        InputSpec::Lando {
            base_lando_id,
            new_lando_id,
            ..
        } => Ok(insufficient_diagnosis(format!(
            "Lando compare base={} new={} — resolve to revision hashes first via perf-alert-cli",
            base_lando_id, new_lando_id
        ))),
    }
}

/// Diagnose by test name + platform: search Perfherder for recent matching alerts.
pub fn diagnose_test_platform(test: &str, platform: &str, verbose: bool) -> Result<Diagnosis> {
    use crate::api::get_json;

    #[derive(serde::Deserialize)]
    struct AlertList {
        results: Vec<serde_json::Value>,
    }

    // Search across perf frameworks for alerts matching this test name
    let frameworks = [13u32, 15, 1, 4];
    let mut alert_ids: Vec<u64> = Vec::new();
    for fw in &frameworks {
        let url = format!(
            "https://treeherder.mozilla.org/api/performance/alertsummary/?status=0&framework={fw}&limit=20"
        );
        if let Ok(list) = get_json::<AlertList>(&url) {
            for v in &list.results {
                if let Some(id) = v.get("id").and_then(|i| i.as_u64()) {
                    alert_ids.push(id);
                }
            }
        }
    }

    // Find the first alert that references our test name
    for id in alert_ids {
        if let Ok(summary) = api::perfherder::fetch_alert_summary(id) {
            let matches_test = summary
                .regressions
                .iter()
                .chain(summary.improvements.iter())
                .any(|r| {
                    r.test.to_lowercase().contains(&test.to_lowercase())
                        && r.platform.to_lowercase().contains(&platform.to_lowercase())
                });
            if matches_test {
                if verbose {
                    eprintln!("Found matching alert: {}", id);
                }
                return diagnose_alert(id, Some(format!("{} on {}", test, platform)), verbose);
            }
        }
    }

    Ok(insufficient_diagnosis(format!(
        "No recent untriaged alerts found for '{}' on '{}'. \
         Try passing an alert ID directly.",
        test, platform
    )))
}

fn diagnose_alert(
    alert_id: u64,
    summary_override: Option<String>,
    verbose: bool,
) -> Result<Diagnosis> {
    if verbose {
        eprintln!("Fetching alert summary {}...", alert_id);
    }

    let summary = api::perfherder::fetch_alert_summary(alert_id)?;
    let signal_type = classify_signal_type(&summary);

    let input_summary = summary_override.unwrap_or_else(|| {
        format!(
            "Alert {} — {} ({}) on {}",
            summary.id, summary.status, summary.framework, summary.repository
        )
    });

    // Try fetching actual failure logs via treeherder-cli for richer pattern matching
    let log_text = summary
        .regressions
        .first()
        .map(|r| logs::treeherder_url(&r.new_push.revision, &r.new_push.repo))
        .and_then(|url| logs::fetch_failure_logs(&url).ok());

    let noise_context = fetch_noise_context(&summary, verbose);
    let existing_bugs = find_existing_bugs(&summary, verbose);
    let failure_rate = compute_failure_rate(&summary, verbose);

    // Match against real log text if available, fall back to test name proxy
    let findings = if let Some(ref log) = log_text {
        find_pattern_matches_in_log(log)
    } else {
        find_pattern_matches_by_name(&summary)
    };

    // Compute overall confidence
    let confidence = compute_confidence(&findings, &failure_rate, &existing_bugs);

    // Build next steps
    let mut next_steps = Vec::new();
    if findings.is_empty() {
        next_steps
            .push("No known pattern matched. Check the full job log for error details.".into());
        next_steps.push(format!(
            "Treeherder: https://treeherder.mozilla.org/perfherder/alerts?id={}",
            alert_id
        ));
    } else {
        for f in &findings {
            next_steps.push(f.next_step.clone());
        }
    }

    if existing_bugs.is_empty() {
        next_steps.push(
            "No existing Bugzilla bug found — consider filing one if this is recurring.".into(),
        );
    } else {
        for b in &existing_bugs {
            next_steps.push(format!(
                "Existing bug: https://bugzilla.mozilla.org/show_bug.cgi?id={} — {}",
                b.id, b.summary
            ));
        }
    }

    // Surface profile URLs for regressions that have them
    for r in summary.regressions.iter().filter(|r| r.has_profile) {
        if let Some(url) = &r.profile_url {
            // Attempt to list artifacts for the profile task to get a direct download URL
            let task_hint = extract_task_id_from_url(url);
            if let Some(task_id) = task_hint {
                if let Ok(artifacts) = api::taskcluster::list_artifacts_for_task(&task_id) {
                    for a in artifacts.iter().filter(|a| a.name.contains("profile")) {
                        next_steps.push(format!("Profile: {}", a.url));
                    }
                }
            } else {
                next_steps.push(format!("Profile available: {}", url));
            }
        }
    }

    if matches!(signal_type, SignalType::SustainedRegression) {
        next_steps.push(
            "This looks like a SUSTAINED REGRESSION (Perfherder t-test verdict). \
             Use 'perf-alert-cli info' to investigate the culprit commit."
                .into(),
        );
    }

    Ok(Diagnosis {
        input_summary,
        signal_type,
        failure_rate,
        findings,
        existing_bugs,
        next_steps,
        confidence,
        noise_context,
    })
}

fn classify_signal_type(summary: &AlertSummary) -> SignalType {
    // Sustained regressions are what Perfherder's t-test flags.
    // Intermittents tend to show up as "investigating" or "untriaged" with
    // non-zero regressions but high variance per-run.
    match summary.status.as_str() {
        "untriaged" | "investigating" => {
            if summary.regressions.is_empty() {
                SignalType::Intermittent
            } else {
                // Has regressions but still untriaged — treat as potentially sustained
                SignalType::SustainedRegression
            }
        }
        "downstream" | "invalid" => SignalType::Intermittent,
        _ => SignalType::Inconclusive,
    }
}

/// Diagnose from raw log text (e.g. from treeherder-cli or a Push input).
fn diagnose_from_log(log_text: &str, input_summary: &str) -> Diagnosis {
    let findings = find_pattern_matches_in_log(log_text);
    let confidence = if findings.is_empty() {
        Confidence::Insufficient
    } else {
        Confidence::Medium
    };
    let mut next_steps: Vec<String> = findings.iter().map(|f| f.next_step.clone()).collect();
    if findings.is_empty() {
        next_steps.push("No known pattern matched in the job log.".into());
    }
    Diagnosis {
        input_summary: input_summary.to_owned(),
        signal_type: SignalType::Intermittent,
        failure_rate: None,
        findings,
        existing_bugs: vec![],
        next_steps,
        confidence,
        noise_context: None,
    }
}

/// Match patterns against actual job log text.
fn find_pattern_matches_in_log(log_text: &str) -> Vec<Finding> {
    let lower = log_text.to_lowercase();
    patterns::PATTERNS
        .iter()
        .filter(|p| p.matches.iter().all(|m| lower.contains(&m.to_lowercase())))
        .map(|p| Finding {
            category: p.category.into(),
            description: p.description.into(),
            root_cause: p.root_cause.into(),
            next_step: p.next_step.into(),
            matched_pattern: Some(p.description.into()),
        })
        .collect()
}

/// Match patterns against test/suite names when no log is available.
fn find_pattern_matches_by_name(summary: &AlertSummary) -> Vec<Finding> {
    let index_context: String = summary
        .regressions
        .iter()
        .flat_map(|r| {
            crate::index::searchfox::search_with_fallback(&r.test, false)
                .unwrap_or_default()
                .into_iter()
                .map(|e| e.name)
        })
        .collect::<Vec<_>>()
        .join(" ");

    let combined = format!(
        "{} {} {} {}",
        summary.framework,
        summary.repository,
        summary
            .regressions
            .iter()
            .map(|r| format!("{} {} {}", r.test, r.suite, r.platform))
            .collect::<Vec<_>>()
            .join(" "),
        index_context,
    )
    .to_lowercase();

    patterns::PATTERNS
        .iter()
        .filter(|p| {
            p.matches
                .iter()
                .all(|m| combined.contains(&m.to_lowercase()))
        })
        .map(|p| Finding {
            category: p.category.into(),
            description: p.description.into(),
            root_cause: p.root_cause.into(),
            next_step: p.next_step.into(),
            matched_pattern: Some(p.description.into()),
        })
        .collect()
}

fn compute_failure_rate(summary: &AlertSummary, verbose: bool) -> Option<FailureRate> {
    // Failure rate requires fetching individual job runs from Treeherder.
    // This is done via the jobs API keyed on push_id.
    if verbose {
        eprintln!("Fetching job runs for push_id {}...", summary.push_id);
    }

    match api::treeherder::fetch_jobs_for_push(summary.push_id, &summary.repository) {
        Ok(jobs) => {
            let framework_jobs: Vec<_> = jobs
                .iter()
                .filter(|j| is_perf_job(&j.job_type_name))
                .collect();

            if framework_jobs.len() < MIN_RUNS_FOR_DIAGNOSIS {
                return None;
            }

            let failures = framework_jobs
                .iter()
                .filter(|j| j.result != "success" && j.result != "completed")
                .count();
            let total = framework_jobs.len();
            let rate = (failures as f64 / total as f64) * 100.0;

            Some(FailureRate {
                failures,
                total_runs: total,
                rate_percent: rate,
            })
        }
        Err(_) => None,
    }
}

fn fetch_noise_context(summary: &AlertSummary, verbose: bool) -> Option<String> {
    // Attempt stmo-cli query for historical noise baseline.
    // Falls back gracefully if stmo-cli is not available.
    use crate::tools::{CliTool, Tool};

    let stmo = CliTool::new("stmo-cli");
    if !stmo.check_available() {
        if verbose {
            eprintln!("stmo-cli not found on PATH — skipping noise context");
        }
        return None;
    }

    let test_name = summary
        .regressions
        .first()
        .map(|r| format!("{}/{}", r.suite, r.test))?;

    match stmo.run(&["query", "--test", &test_name, "--json"]) {
        Ok(out) if out.exit_code == 0 => Some(out.stdout.trim().to_owned()),
        _ => None,
    }
}

fn find_existing_bugs(summary: &AlertSummary, verbose: bool) -> Vec<ExistingBug> {
    let test_name = match summary.regressions.first().or(summary.improvements.first()) {
        Some(r) => format!("{}/{}", r.suite, r.test),
        None => return vec![],
    };

    if verbose {
        eprintln!(
            "Searching Bugzilla for existing bugs matching {:?}...",
            test_name
        );
    }

    match api::bugzilla::search_intermittent_bugs(&test_name) {
        Ok(bugs) => bugs
            .into_iter()
            .take(5)
            .map(|b| ExistingBug {
                id: b.id,
                summary: b.summary,
                status: b.status,
                resolution: b.resolution,
            })
            .collect(),
        Err(_) => vec![],
    }
}

fn compute_confidence(
    findings: &[Finding],
    failure_rate: &Option<FailureRate>,
    existing_bugs: &[ExistingBug],
) -> Confidence {
    let has_pattern = !findings.is_empty();
    let has_rate = failure_rate.is_some();
    let has_bug = !existing_bugs.is_empty();

    match (has_pattern, has_rate, has_bug) {
        (true, true, _) => Confidence::High,
        (true, false, _) | (false, true, true) => Confidence::Medium,
        (false, false, true) => Confidence::Low,
        _ => Confidence::Insufficient,
    }
}

fn is_perf_job(name: &str) -> bool {
    let n = name.to_lowercase();
    n.contains("browsertime")
        || n.contains("raptor")
        || n.contains("awsy")
        || n.contains("talos")
        || n.contains("mozperftest")
}

/// Extract a Taskcluster task ID from a profile artifact URL.
fn extract_task_id_from_url(url: &str) -> Option<String> {
    // TC artifact URLs are: .../task/{task_id}/artifacts/...
    let after_task = url.split("/task/").nth(1)?;
    let task_id = after_task.split('/').next()?;
    if task_id.len() > 10 {
        Some(task_id.to_owned())
    } else {
        None
    }
}

fn insufficient_diagnosis(message: String) -> Diagnosis {
    Diagnosis {
        input_summary: message.clone(),
        signal_type: SignalType::Inconclusive,
        failure_rate: None,
        findings: vec![],
        existing_bugs: vec![],
        next_steps: vec![message],
        confidence: Confidence::Insufficient,
        noise_context: None,
    }
}
