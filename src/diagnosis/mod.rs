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
    /// Fix type hint for patch generation.
    pub fix_type: String,
    /// Platforms where this failure commonly occurs. Empty = all.
    pub platform_hints: Vec<String>,
    /// Example Bugzilla bug demonstrating this pattern.
    pub example_bug: Option<u64>,
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
        InputSpec::Alert { alert_id } => {
            // Try Perfherder first. If it 404s, the number might be a Bugzilla bug ID —
            // Perfherder alert IDs are typically 5 digits (~50000s), Bugzilla IDs are 7+ digits.
            match diagnose_alert(*alert_id, None, verbose) {
                Ok(d) => Ok(d),
                Err(e) if e.to_string().contains("404") => {
                    if *alert_id > 999_999 {
                        // Looks like a Bugzilla bug ID — retry as Bug input
                        if verbose {
                            eprintln!("Alert {} not found on Perfherder — retrying as Bugzilla bug ID...", alert_id);
                        }
                        let bug_spec = InputSpec::Bug { bug_id: *alert_id };
                        diagnose(&bug_spec, verbose)
                    } else {
                        Err(e.context(format!(
                            "Alert {} not found on Perfherder. \
                             If this is a Bugzilla bug number, pass the full URL: \
                             https://bugzilla.mozilla.org/show_bug.cgi?id={}",
                            alert_id, alert_id
                        )))
                    }
                }
                Err(e) => Err(e),
            }
        }
        InputSpec::Push { push } => {
            // Fetch logs directly from Treeherder via treeherder-cli
            let url = logs::treeherder_url(&push.revision, &push.repo);
            let log_text = logs::fetch_failure_logs(&url)?;
            let input_summary = format!("Push {} ({})", push.revision, push.repo);
            Ok(diagnose_from_log(&log_text, &input_summary))
        }
        InputSpec::Bug { bug_id } => {
            let bug = api::bugzilla::fetch_bug(*bug_id)?;

            // Detect CaR from bug summary before hitting Perfherder.
            // CaR bugs look like: "Perma [custom-car] subprocess.CalledProcessError..."
            let summary_lower = bug.summary.to_lowercase();
            if summary_lower.contains("custom-car") || summary_lower.contains("[car]")
                || summary_lower.contains("chromium-as-release")
            {
                let short = bug.summary.chars().take(80).collect::<String>();
                return Ok(insufficient_diagnosis_with_steps(
                    format!("Bug {} [CaR]: {}", bug.id, short),
                    vec![
                        format!("Bug {}: {} — this is a CaR (Chromium-as-Release) failure.", bug.id, bug.summary),
                        "Use car-mechanic-cli — it has the full CaR failure pattern database.".into(),
                        "Get a Treeherder URL from the bug, then run: car-mechanic diagnose --url '<url>'".into(),
                        "Install: cargo install --git https://github.com/92kns/car-mechanic-cli".into(),
                    ],
                ));
            }

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

/// Detect whether a suite/test name looks like a CaR (Chromium-as-Release) job.
fn is_car_test(suite: &str, test: &str) -> bool {
    let combined = format!("{suite} {test}").to_lowercase();
    combined.contains("custom-car")
        || combined.contains("chromium-as-release")
        || combined.contains("-car-")
        || combined.starts_with("car-")
}

/// Delegate to car-mechanic-cli for CaR-related failures.
fn diagnose_car(alert_id: u64, suite: &str, test: &str, url: &str) -> Diagnosis {
    use crate::tools::{CliTool, Tool};

    let car = CliTool::new("car-mechanic");
    let has_car = car.check_available();

    let mut next_steps = vec![format!(
        "This is a CaR (Chromium-as-Release) test: {suite}/{test}. \
             Use car-mechanic-cli for diagnosis — it has the full CaR failure pattern database."
    )];

    if has_car {
        // Try to run car-mechanic diagnose directly
        match car.run(&["diagnose", "--url", url, "--json"]) {
            Ok(out) if out.exit_code == 0 && !out.stdout.trim().is_empty() => {
                return Diagnosis {
                    input_summary: format!("CaR Alert {} — {suite}/{test}", alert_id),
                    signal_type: SignalType::Inconclusive,
                    failure_rate: None,
                    findings: vec![Finding {
                        category: "car_delegated".into(),
                        description: "Delegated to car-mechanic-cli".into(),
                        root_cause: out.stdout.trim().chars().take(500).collect(),
                        next_step: "See car-mechanic output above for fix steps.".into(),
                        matched_pattern: None,
                        fix_type: "CodeFix".into(),
                        platform_hints: vec![],
                        example_bug: None,
                    }],
                    existing_bugs: vec![],
                    next_steps: vec!["See car-mechanic-cli output above.".into()],
                    confidence: Confidence::Medium,
                    noise_context: None,
                };
            }
            _ => {
                next_steps.push(format!("Run: car-mechanic diagnose --url '{url}'"));
            }
        }
    } else {
        next_steps.push(
            "Install car-mechanic-cli: cargo install --git https://github.com/92kns/car-mechanic-cli".into()
        );
        next_steps.push(format!("Then: car-mechanic diagnose --url '{url}'"));
    }

    insufficient_diagnosis_with_steps(
        format!(
            "CaR Alert {} — {suite}/{test} (delegated to car-mechanic-cli)",
            alert_id
        ),
        next_steps,
    )
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

    // CaR tests belong to car-mechanic-cli — hand off early.
    if let Some(r) = summary.regressions.first().or(summary.improvements.first()) {
        if is_car_test(&r.suite, &r.test) {
            let url = logs::treeherder_url(&r.new_push.revision, &r.new_push.repo);
            return Ok(diagnose_car(alert_id, &r.suite, &r.test, &url));
        }
    }

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
        next_steps.push(format!(
            "This looks like a SUSTAINED REGRESSION (Perfherder t-test verdict). \
             Run `perftest-brain commits {}` to rank commits by relevance.",
            alert_id
        ));
    }

    // Suggest stmo-cli for historical noise context — agents call it directly
    let test_name = summary
        .regressions
        .first()
        .or(summary.improvements.first())
        .map(|r| format!("{}/{}", r.suite, r.test));
    if let Some(ref name) = test_name {
        next_steps.push(format!(
            "For historical noise context: run `stmo-cli execute <query-id> --format json --param test={}` \
             (replace <query-id> with a STMO query that queries Perfherder signal history)",
            name
        ));
    }

    Ok(Diagnosis {
        input_summary,
        signal_type,
        failure_rate,
        findings,
        existing_bugs,
        next_steps,
        confidence,
        noise_context: None,
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

fn pattern_to_finding(p: &patterns::Pattern) -> Finding {
    Finding {
        category: p.category.into(),
        description: p.description.into(),
        root_cause: p.root_cause.into(),
        next_step: p.next_step.into(),
        matched_pattern: Some(p.description.into()),
        fix_type: format!("{:?}", p.fix_type),
        platform_hints: p.platform_hints.iter().map(|s| s.to_string()).collect(),
        example_bug: p.example_bug,
    }
}

/// Match patterns against actual job log text.
fn find_pattern_matches_in_log(log_text: &str) -> Vec<Finding> {
    let lower = log_text.to_lowercase();
    patterns::PATTERNS
        .iter()
        .filter(|p| p.matches.iter().all(|m| lower.contains(&m.to_lowercase())))
        .map(pattern_to_finding)
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
        .map(pattern_to_finding)
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
    insufficient_diagnosis_with_steps(message.clone(), vec![message])
}

fn insufficient_diagnosis_with_steps(summary: String, next_steps: Vec<String>) -> Diagnosis {
    Diagnosis {
        input_summary: summary,
        signal_type: SignalType::Inconclusive,
        failure_rate: None,
        findings: vec![],
        existing_bugs: vec![],
        next_steps,
        confidence: Confidence::Insufficient,
        noise_context: None,
    }
}
