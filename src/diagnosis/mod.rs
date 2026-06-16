pub mod patterns;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::api;
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

/// Outcome of trying to fetch enough run history.
pub enum RunHistoryResult {
    Sufficient(Vec<JobRun>),
    /// Not enough runs — user must retrigger N more times.
    NeedRetrigger { have: usize, need: usize },
}

/// One job run record.
#[derive(Debug)]
pub struct JobRun {
    pub result: String,
    pub job_type_name: String,
    pub platform: String,
}

/// Diagnose a resolved input spec.
///
/// Returns `Err` for genuine errors, and a `Diagnosis` with `confidence: Insufficient`
/// when there is not enough data (caller should surface the retrigger recommendation).
pub fn diagnose(spec: &InputSpec, verbose: bool) -> Result<Diagnosis> {
    match spec {
        InputSpec::Alert { alert_id } => diagnose_alert(*alert_id, verbose),
        InputSpec::Push { push } => {
            Ok(insufficient_diagnosis(format!(
                "Push {} — use 'info' to get the alert ID, then 'diagnose <alert-id>'",
                push.revision
            )))
        }
        InputSpec::Bug { bug_id } => {
            // Look up the bug, then suggest the alert route
            let bug = api::bugzilla::fetch_bug(*bug_id)?;
            Ok(insufficient_diagnosis(format!(
                "Bug {}: {} — find the Perfherder alert ID linked from this bug and pass it to diagnose",
                bug.id, bug.summary
            )))
        }
        InputSpec::PerfCompare { base, new } => {
            Ok(insufficient_diagnosis(format!(
                "PerfCompare base={} new={} — pass the Perfherder alert ID for a full diagnosis",
                base.revision, new.revision
            )))
        }
        InputSpec::Lando { base_lando_id, new_lando_id, .. } => {
            Ok(insufficient_diagnosis(format!(
                "Lando compare base={} new={} — pass the Perfherder alert ID for a full diagnosis",
                base_lando_id, new_lando_id
            )))
        }
    }
}

fn diagnose_alert(alert_id: u64, verbose: bool) -> Result<Diagnosis> {
    if verbose {
        eprintln!("Fetching alert summary {}...", alert_id);
    }

    let summary = api::perfherder::fetch_alert_summary(alert_id)?;

    // Classify signal type based on the alert status
    let signal_type = classify_signal_type(&summary);

    let input_summary = format!(
        "Alert {} — {} ({}) on {}",
        summary.id, summary.status, summary.framework, summary.repository
    );

    // Attempt stmo-cli historical signal quality check
    let noise_context = fetch_noise_context(&summary, verbose);

    // Attempt Bugzilla lookup for existing bugs
    let existing_bugs = find_existing_bugs(&summary, verbose);

    // Try to fetch job run history from Treeherder for failure rate
    let failure_rate = compute_failure_rate(&summary, verbose);

    // Match failure patterns
    let findings = find_pattern_matches(&summary);

    // Compute overall confidence
    let confidence = compute_confidence(&findings, &failure_rate, &existing_bugs);

    // Build next steps
    let mut next_steps = Vec::new();
    if findings.is_empty() {
        next_steps.push("No known pattern matched. Check the full job log for error details.".into());
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
        next_steps.push("No existing Bugzilla bug found — consider filing one if this is recurring.".into());
    } else {
        for b in &existing_bugs {
            next_steps.push(format!(
                "Existing bug: https://bugzilla.mozilla.org/show_bug.cgi?id={} — {}",
                b.id, b.summary
            ));
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

fn find_pattern_matches(summary: &AlertSummary) -> Vec<Finding> {
    // For now we don't have the raw log text from the API (that requires
    // fetching Taskcluster job logs — Phase 3 stubs this, Phase 4+ fetches real logs).
    // We match against the test/suite names as a proxy.
    let combined = format!(
        "{} {} {}",
        summary.framework,
        summary.repository,
        summary
            .regressions
            .iter()
            .map(|r| format!("{} {} {}", r.test, r.suite, r.platform))
            .collect::<Vec<_>>()
            .join(" ")
    )
    .to_lowercase();

    patterns::PATTERNS
        .iter()
        .filter(|p| p.matches.iter().all(|m| combined.contains(&m.to_lowercase())))
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
        eprintln!(
            "Fetching job runs for push_id {}...",
            summary.push_id
        );
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
        eprintln!("Searching Bugzilla for existing bugs matching {:?}...", test_name);
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
