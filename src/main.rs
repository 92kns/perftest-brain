use clap::{Parser, Subcommand};
use std::path::PathBuf;

mod analysis;
mod api;
mod checkout;
mod diagnosis;
mod doctor;
mod index;
mod input;
mod logs;
mod patch;
mod sheriff;
mod tools;
mod types;

use types::InputSpec;

const AGENTS_MD: &str = include_str!("../AGENTS.md");

#[derive(Parser)]
#[command(
    name = "perftest-brain",
    version,
    about = "Embedded Firefox perf-test engineer: diagnose, patch, and triage intermittent failures"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Output results as JSON. [INP-02]
    #[arg(long, global = true)]
    json: bool,

    /// Increase verbosity (-v debug, -vv subprocess output).
    #[arg(short, long, global = true, action = clap::ArgAction::Count)]
    verbose: u8,

    /// Override Firefox checkout auto-detection. [INP-03]
    #[arg(long, value_name = "PATH", global = true)]
    checkout_path: Option<PathBuf>,
}

#[derive(Subcommand)]
enum Commands {
    /// Diagnose an intermittent failure from a signal (Treeherder job, test name, alert).
    Diagnose {
        /// Failure signal to diagnose.
        input: Option<String>,
    },
    /// Generate a fix patch for a diagnosed failure.
    Patch {
        /// Failure signal or diagnosis to patch.
        input: Option<String>,
    },
    /// Sheriff triage classification for a failure signal.
    Sheriff {
        /// Failure signal to triage.
        input: Option<String>,
    },
    /// Groom the alert/triage backlog.
    Groom,
    /// Run environment and tool health checks.
    Doctor {
        /// Harness to check (raptor, mozperftest).
        harness: Option<String>,
    },
    /// Update perftest-brain to the latest version from GitHub.
    ///
    /// Equivalent to: cargo install --force --git https://github.com/92kns/perftest-brain
    Update,
    /// Rebuild the local test-metadata index from the Firefox checkout.
    Reindex,
    /// Show information about a signal or the current checkout.
    Info {
        /// Signal: alert ID, Treeherder URL, Bugzilla URL, or revision hash.
        input: Option<String>,
    },
    /// Show commits in a regression window ranked by relevance to the regressed test.
    ///
    /// Equivalent to: perf-alert-cli commits <input>
    Commits {
        /// Alert ID, Treeherder URL, PerfCompare URL, or revision hash.
        input: String,
    },
    /// List Gecko profiler profiles available for a push.
    ///
    /// Equivalent to: perf-alert-cli profiles <input>
    Profiles {
        /// Alert ID, Treeherder URL, or revision hash.
        input: String,
        /// Filter to jobs matching this test name substring.
        #[arg(long)]
        test: Option<String>,
    },
    /// Print the AI agent usage guide (AGENTS.md) to stdout.
    ///
    /// Pipe to feed directly to an AI agent as context:
    ///   perftest-brain agents | <your-ai-tool>
    Agents,
}

fn main() {
    let cli = Cli::parse();
    let json = cli.json;

    match run(cli) {
        Ok(()) => {}
        Err(e) => {
            if json {
                eprintln!(
                    r#"{{"error":"{}","exit_code":1}}"#,
                    escape_json(&format!("{e:#}"))
                );
            } else {
                eprintln!("error: {e:#}");
            }
            std::process::exit(1);
        }
    }
}

fn escape_json(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ => out.push(c),
        }
    }
    out
}

fn run(cli: Cli) -> anyhow::Result<()> {
    let json = cli.json;
    let verbose = cli.verbose;

    // Resolve checkout only for commands that need it.
    let needs_checkout = matches!(
        cli.command,
        Commands::Patch { .. } | Commands::Doctor { .. } | Commands::Reindex
    );

    let checkout = if needs_checkout {
        let co = checkout::resolve(
            cli.checkout_path.as_deref(),
            std::env::var("PERFTEST_BRAIN_CHECKOUT").ok(),
        )?;
        if verbose > 0 {
            eprintln!("checkout: {} ({:?})", co.path.display(), co.vcs);
        }
        Some(co)
    } else {
        None
    };

    match cli.command {
        Commands::Agents => {
            print!("{}", AGENTS_MD);
            Ok(())
        }
        Commands::Update => cmd_self_update(),
        Commands::Info { input } => cmd_info(input.as_deref(), json),
        Commands::Commits { input } => cmd_commits(&input, json, verbose),
        Commands::Profiles { input, test } => cmd_profiles(&input, test.as_deref(), json, verbose),
        Commands::Diagnose { input } => cmd_diagnose(input.as_deref(), json, verbose),
        Commands::Patch { input } => {
            cmd_patch(input.as_deref(), checkout.as_ref().unwrap(), json, verbose)
        }
        Commands::Sheriff { input } => cmd_sheriff(input.as_deref(), json, verbose),
        Commands::Groom => cmd_groom(json, verbose),
        Commands::Doctor { harness } => cmd_doctor(
            harness.as_deref(),
            checkout.as_ref().unwrap(),
            json,
            verbose,
        ),
        Commands::Reindex => cmd_reindex(checkout.as_ref().unwrap(), json, verbose),
    }
}

const REPO_URL: &str = "https://github.com/92kns/perftest-brain";

fn cmd_self_update() -> anyhow::Result<()> {
    const CURRENT: &str = env!("CARGO_PKG_VERSION");

    let latest = fetch_latest_tag().unwrap_or_else(|e| {
        eprintln!("warn: could not check latest version: {}", e);
        None
    });

    match &latest {
        Some(tag) if tag.trim_start_matches('v') == CURRENT => {
            println!("Already up to date (v{}).", CURRENT);
            return Ok(());
        }
        Some(tag) => println!("Updating v{} → {} ...", CURRENT, tag),
        None => println!("Updating perftest-brain (current: v{})...", CURRENT),
    }

    let mut args = vec!["install", "--force", "--git", REPO_URL];
    let tag_owned;
    if let Some(ref tag) = latest {
        tag_owned = tag.clone();
        args.extend_from_slice(&["--tag", &tag_owned]);
    }

    let status = std::process::Command::new("cargo").args(&args).status()?;
    if status.success() {
        println!("Updated successfully.");
        Ok(())
    } else {
        anyhow::bail!("cargo install exited with status {}", status)
    }
}

fn fetch_latest_tag() -> anyhow::Result<Option<String>> {
    #[derive(serde::Deserialize)]
    struct Tag {
        name: String,
    }

    let url = "https://api.github.com/repos/92kns/perftest-brain/tags";
    let body = ureq::get(url)
        .set("Accept", "application/vnd.github.v3+json")
        .set(
            "User-Agent",
            concat!("perftest-brain/", env!("CARGO_PKG_VERSION")),
        )
        .call()?
        .into_string()?;

    let tags: Vec<Tag> = serde_json::from_str(&body)?;
    Ok(tags.into_iter().next().map(|t| t.name))
}

fn cmd_patch(
    raw_input: Option<&str>,
    checkout: &checkout::CheckoutRoot,
    json: bool,
    verbose: u8,
) -> anyhow::Result<()> {
    let raw = raw_input.ok_or_else(|| {
        anyhow::anyhow!("Usage: perftest-brain patch <alert-id | URL | revision>")
    })?;

    let spec = input::parse_input(raw)?;

    // --yes is implicit when --json is passed (non-interactive mode)
    let auto_apply = json;

    if !auto_apply {
        eprintln!("Analyzing signal for patchable issues...");
    }

    let result = patch::patch(&spec, checkout, auto_apply, verbose > 0)?;

    if json {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        if result.applied.is_empty() && result.skipped.is_empty() {
            println!("No patches to apply.");
        }
        for action in &result.applied {
            println!("Applied: {}", action.description);
        }
        for skip in &result.skipped {
            println!("Skipped: {}", skip);
        }
        println!("\nNext steps:");
        for step in &result.next_steps {
            println!("  • {}", step);
        }
    }

    Ok(())
}

fn cmd_sheriff(raw_input: Option<&str>, json: bool, verbose: u8) -> anyhow::Result<()> {
    let raw = raw_input
        .ok_or_else(|| anyhow::anyhow!("Usage: perftest-brain sheriff <alert-id | URL>"))?;
    let spec = input::parse_input(raw)?;
    let analysis = sheriff::analyze(&spec, verbose > 0)?;

    if json {
        println!("{}", serde_json::to_string_pretty(&analysis)?);
    } else {
        println!(
            "{} | Backout: {}",
            analysis.tier,
            if analysis.backout_recommended {
                "YES"
            } else {
                "No"
            }
        );
        println!(
            "Framework: {} | Worst regression: {:.1}%",
            analysis.framework, analysis.worst_regression_pct
        );
        println!("Test: {}", analysis.test_summary);
        println!("Classification: {}", analysis.classification_reasoning);
        println!("Backout reasoning: {}", analysis.backout_reasoning);
        if !analysis.affected_platforms.is_empty() {
            println!("Platforms: {}", analysis.affected_platforms.join(", "));
        }
        if let Some(id) = analysis.alert_id {
            println!(
                "Treeherder: https://treeherder.mozilla.org/perfherder/alerts?id={}",
                id
            );
        }
    }
    Ok(())
}

fn cmd_groom(json: bool, verbose: u8) -> anyhow::Result<()> {
    // Fetch recent untriaged alerts from Perfherder
    // For Phase 6, we fetch from the default perfherder alert endpoint
    use crate::api::get_json;

    #[derive(serde::Deserialize)]
    struct AlertList {
        results: Vec<serde_json::Value>,
    }

    // Fetch untriaged alerts across all perf frameworks (browsertime=13, mozperftest=15, talos=1, awsy=4)
    let frameworks = [13u32, 15, 1, 4];
    if verbose > 0 {
        eprintln!("Fetching untriaged alerts across all perf frameworks...");
    }

    let mut alert_ids: Vec<u64> = Vec::new();
    for fw in &frameworks {
        let url = format!(
            "https://treeherder.mozilla.org/api/performance/alertsummary/?status=0&framework={fw}&limit=10"
        );
        if let Ok(list) = get_json::<AlertList>(&url) {
            alert_ids.extend(list.results.iter().filter_map(|v| v.get("id")?.as_u64()));
        }
    }

    if alert_ids.is_empty() {
        println!("No untriaged alerts found.");
        return Ok(());
    }

    let entries = sheriff::groom(&alert_ids, verbose > 0)?;

    if json {
        println!("{}", serde_json::to_string_pretty(&entries)?);
    } else {
        println!("Groomed {} alerts (sorted by priority):", entries.len());
        println!(
            "{:<8} {:<8} {:<12} {:<10} Test",
            "Alert", "Tier", "Framework", "Score"
        );
        println!("{}", "-".repeat(72));
        for e in &entries {
            println!(
                "{:<8} {:<8} {:<12} {:<10.1} {}",
                e.alert_id,
                format!("{}", e.tier),
                e.framework,
                e.score,
                e.test_summary
            );
            if let Some(owner) = &e.suggested_owner {
                println!("         Suggested owner: {}", owner);
            }
        }
    }
    Ok(())
}

fn cmd_doctor(
    harness: Option<&str>,
    checkout: &checkout::CheckoutRoot,
    json: bool,
    verbose: u8,
) -> anyhow::Result<()> {
    let harness = harness
        .ok_or_else(|| anyhow::anyhow!("Usage: perftest-brain doctor <raptor|mozperftest>"))?;

    let report = doctor::run_doctor(harness, &checkout.path, verbose > 0)?;

    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!(
            "Doctor report for {} — Overall: {}",
            report.harness, report.overall
        );
        println!("{}", "-".repeat(60));
        for check in &report.checks {
            let icon = match check.status {
                doctor::CheckStatus::Ok => "✓",
                doctor::CheckStatus::Warn => "⚠",
                doctor::CheckStatus::Fail => "✗",
            };
            println!(
                "{} [{}] {}: {}",
                icon, check.status, check.name, check.detail
            );
            if let Some(hint) = &check.fix_hint {
                println!("  Fix: {}", hint);
            }
        }
    }
    Ok(())
}

fn cmd_reindex(checkout: &checkout::CheckoutRoot, json: bool, verbose: u8) -> anyhow::Result<()> {
    if verbose > 0 {
        if let Ok(prev) = index::index_stats() {
            if let Some(ts) = prev.last_updated {
                eprintln!(
                    "Previous index: {} tests, {} tasks (updated {}s ago)",
                    prev.test_count,
                    prev.task_count,
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs()
                        .saturating_sub(ts)
                );
            }
        }
    }
    eprintln!(
        "Indexing Firefox checkout at {}...",
        checkout.path.display()
    );
    let stats = index::update_index(&checkout.path, verbose > 0)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&stats)?);
    } else {
        println!(
            "Index updated: {} tests, {} taskcluster files",
            stats.test_count, stats.task_count
        );
        println!("Database: {}", stats.db_path.display());
    }
    Ok(())
}

fn cmd_commits(raw_input: &str, json: bool, verbose: u8) -> anyhow::Result<()> {
    let spec = input::parse_input(raw_input)?;

    // Resolve to a base/new push pair
    let (base_push, new_push, suite, test) = match &spec {
        types::InputSpec::Alert { alert_id } => {
            if verbose > 0 {
                eprintln!("Fetching alert {}...", alert_id);
            }
            let summary = api::perfherder::fetch_alert_summary(*alert_id)?;
            let first = summary
                .regressions
                .first()
                .or(summary.improvements.first())
                .ok_or_else(|| anyhow::anyhow!("Alert {} has no alerts", alert_id))?;
            let suite = first.suite.clone();
            let test = first.test.clone();
            (first.base_push.clone(), first.new_push.clone(), suite, test)
        }
        types::InputSpec::PerfCompare { base, new } => {
            (base.clone(), new.clone(), String::new(), String::new())
        }
        types::InputSpec::Push { .. } => {
            anyhow::bail!(
                "Pass an alert ID or PerfCompare URL to get a regression window. \
                           A single push revision has no base to compare against."
            )
        }
        _ => anyhow::bail!("Pass an alert ID or PerfCompare URL for commit analysis."),
    };

    if verbose > 0 {
        eprintln!(
            "Fetching commits {} → {}...",
            base_push.revision, new_push.revision
        );
    }

    let commits = api::pushlog::fetch_commit_window(&base_push, &new_push)?;
    let ranked = analysis::culprit::rank_commits(commits, &suite, &test);

    if json {
        println!("{}", serde_json::to_string_pretty(&ranked)?);
    } else {
        print!(
            "{}",
            analysis::culprit::format_ranked(&ranked, &suite, &test)
        );
    }

    Ok(())
}

fn cmd_profiles(
    raw_input: &str,
    test_filter: Option<&str>,
    json: bool,
    verbose: u8,
) -> anyhow::Result<()> {
    let spec = input::parse_input(raw_input)?;

    let alert_id = match &spec {
        types::InputSpec::Alert { alert_id } => *alert_id,
        _ => anyhow::bail!(
            "Pass an alert ID for profile lookup. \
             Use `perftest-brain info <url>` to find the alert ID from a Treeherder URL."
        ),
    };

    if verbose > 0 {
        eprintln!("Fetching alert {}...", alert_id);
    }
    let summary = api::perfherder::fetch_alert_summary(alert_id)?;

    let all_alerts: Vec<_> = summary
        .regressions
        .iter()
        .chain(summary.improvements.iter())
        .filter(|r| {
            test_filter
                .map(|f| r.test.to_lowercase().contains(&f.to_lowercase()))
                .unwrap_or(true)
        })
        .collect();

    if all_alerts.is_empty() {
        println!("No alerts found for alert summary {}", alert_id);
        return Ok(());
    }

    // Collect task IDs directly from alert metadata — no jobs API call needed.
    // Dedup by task_id since multiple alerts may share the same task.
    let mut seen = std::collections::HashSet::new();
    let task_ids: Vec<(String, String, String)> = all_alerts
        .iter()
        .filter_map(|r| {
            r.task_id
                .as_ref()
                .map(|t| (t.clone(), r.test.clone(), r.platform.clone()))
        })
        .filter(|(t, _, _)| seen.insert(t.clone()))
        .collect();

    if task_ids.is_empty() {
        println!(
            "Alert {} has no Taskcluster metadata — jobs may still be running.",
            alert_id
        );
        println!("Profiles are only available for completed jobs.");
        return Ok(());
    }

    if verbose > 0 {
        eprintln!(
            "Checking {} task(s) for profile artifacts...",
            task_ids.len()
        );
    }

    let mut found_profiles: Vec<serde_json::Value> = Vec::new();

    for (task_id, test, platform) in &task_ids {
        let artifacts = api::taskcluster::list_artifacts_for_task(task_id).unwrap_or_default();
        for artifact in artifacts {
            if artifact.name.contains("profile") || artifact.name.contains("profiler") {
                found_profiles.push(serde_json::json!({
                    "test": test,
                    "platform": platform,
                    "task_id": task_id,
                    "artifact": artifact.name,
                    "url": artifact.url,
                }));
            }
        }
    }

    if found_profiles.is_empty() {
        println!("No profiles found for alert {}.", alert_id);
        println!("Profiles require the job to have run with --gecko-profile flag.");
        println!("Checked {} task(s).", task_ids.len());
        return Ok(());
    }

    if json {
        println!("{}", serde_json::to_string_pretty(&found_profiles)?);
    } else {
        println!(
            "Found {} profile(s) for alert {}:",
            found_profiles.len(),
            alert_id
        );
        for p in &found_profiles {
            println!(
                "\n  [{}] {}",
                p["platform"].as_str().unwrap_or(""),
                p["test"].as_str().unwrap_or("")
            );
            println!("  {}", p["url"].as_str().unwrap_or(""));
        }
        println!("\nEngineers: load with profiler-cli (ships with your Firefox checkout):");
        println!("  profiler-cli load <url>");
        println!("\nAI agents: run `profiler-cli load <url>` directly then analyze the output.");
    }

    Ok(())
}

fn cmd_diagnose(raw_input: Option<&str>, json: bool, verbose: u8) -> anyhow::Result<()> {
    let raw = raw_input.ok_or_else(|| {
        anyhow::anyhow!(
            "Usage: perftest-brain diagnose <alert-id | URL | revision | \"test-name platform\">"
        )
    })?;

    // Try "test-name platform" format first (e.g. "raptor-speedometer linux64")
    if let Some(tp) = input::try_parse_test_platform(raw) {
        eprintln!(
            "Searching for recent failures of {} on {}...",
            tp.test, tp.platform
        );
        // Find matching alerts via groom, filter by test name
        let diag = diagnosis::diagnose_test_platform(&tp.test, &tp.platform, verbose > 0)?;
        return if json {
            println!("{}", serde_json::to_string_pretty(&diag)?);
            Ok(())
        } else {
            print_diagnosis(&diag);
            Ok(())
        };
    }

    let spec = input::parse_input(raw)?;
    let diag = diagnosis::diagnose(&spec, verbose > 0)?;

    if json {
        println!("{}", serde_json::to_string_pretty(&diag)?);
    } else {
        print_diagnosis(&diag);
    }

    Ok(())
}

fn print_diagnosis(diag: &diagnosis::Diagnosis) {
    println!("Signal: {}", diag.input_summary);
    println!(
        "Type: {:?} | Confidence: {:?}",
        diag.signal_type, diag.confidence
    );

    if let Some(fr) = &diag.failure_rate {
        println!(
            "Failure rate: {}/{} runs ({:.1}%)",
            fr.failures, fr.total_runs, fr.rate_percent
        );
    }

    if !diag.findings.is_empty() {
        println!("\nFindings:");
        for f in &diag.findings {
            println!("  [{}] {}", f.category, f.description);
            println!("  Root cause: {}", f.root_cause);
        }
    }

    if let Some(noise) = &diag.noise_context {
        println!("\nNoise context:\n{}", noise);
    }

    if !diag.existing_bugs.is_empty() {
        println!("\nExisting bugs:");
        for b in &diag.existing_bugs {
            println!("  Bug {}: {} [{}]", b.id, b.summary, b.status);
        }
    }

    println!("\nNext steps:");
    for step in &diag.next_steps {
        println!("  • {}", step);
    }
}

fn cmd_info(raw_input: Option<&str>, json: bool) -> anyhow::Result<()> {
    let raw = raw_input
        .ok_or_else(|| anyhow::anyhow!("Usage: perftest-brain info <alert-id | URL | revision>"))?;

    let spec = input::parse_input(raw)?;

    match &spec {
        InputSpec::Alert { alert_id } => {
            let summary = api::perfherder::fetch_alert_summary(*alert_id)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&summary)?);
            } else {
                println!(
                    "Alert {}: {} ({})",
                    summary.id, summary.status, summary.framework
                );
                println!("Repository: {}", summary.repository);
                println!("Regressions: {}", summary.regressions.len());
                println!("Improvements: {}", summary.improvements.len());
                for r in &summary.regressions {
                    println!(
                        "  [REGRESSION] {}/{} on {} ({:+.1}%)",
                        r.suite, r.test, r.platform, r.delta_percent
                    );
                }
                for r in &summary.improvements {
                    println!(
                        "  [improvement] {}/{} on {} ({:+.1}%)",
                        r.suite, r.test, r.platform, r.delta_percent
                    );
                }
            }
        }
        InputSpec::Bug { bug_id } => {
            let bug = api::bugzilla::fetch_bug(*bug_id)?;
            let alerts =
                api::perfherder::fetch_alert_summaries_for_bug(*bug_id).unwrap_or_default();
            if json {
                #[derive(serde::Serialize)]
                struct BugInfo<'a> {
                    bug: &'a api::bugzilla::Bug,
                    alert_count: usize,
                }
                println!(
                    "{}",
                    serde_json::to_string_pretty(&BugInfo {
                        bug: &bug,
                        alert_count: alerts.len()
                    })?
                );
            } else {
                println!("Bug {}: {}", bug.id, bug.summary);
                println!("Status: {} {}", bug.status, bug.resolution);
                if !alerts.is_empty() {
                    println!("Linked alerts: {}", alerts.len());
                    for a in alerts.iter().take(5) {
                        println!("  Alert {}: {} ({})", a.id, a.status, a.framework);
                    }
                }
            }
        }
        InputSpec::Push { push } => {
            if json {
                println!("{}", serde_json::to_string_pretty(&spec)?);
            } else {
                println!("Push: {} ({})", push.revision, push.repo);
                println!(
                    "Treeherder: https://treeherder.mozilla.org/jobs?repo={}&revision={}",
                    push.repo, push.revision
                );
            }
        }
        InputSpec::PerfCompare { base, new } => {
            if json {
                println!("{}", serde_json::to_string_pretty(&spec)?);
            } else {
                println!("PerfCompare:");
                println!("  Base: {} ({})", base.revision, base.repo);
                println!("  New:  {} ({})", new.revision, new.repo);
            }
        }
        InputSpec::Lando {
            base_lando_id,
            new_lando_id,
            base_repo,
            new_repo,
        } => {
            if json {
                println!("{}", serde_json::to_string_pretty(&spec)?);
            } else {
                println!("Lando compare:");
                println!("  Base: {} ({})", base_lando_id, base_repo);
                println!("  New:  {} ({})", new_lando_id, new_repo);
            }
        }
    }

    Ok(())
}
