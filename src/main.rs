use clap::{Parser, Subcommand};
use std::path::PathBuf;

mod api;
mod checkout;
mod diagnosis;
mod doctor;
mod index;
mod input;
mod patch;
mod sheriff;
mod tools;
mod types;

use types::InputSpec;

const AGENTS_MD: &str = include_str!("../AGENTS.md");
const NOT_IMPLEMENTED: &str = "Not yet implemented — coming in a future phase";

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
    /// Update the local test-metadata index.
    Update,
    /// Show information about a signal or the current checkout.
    Info {
        /// Signal: alert ID, Treeherder URL, Bugzilla URL, or revision hash.
        input: Option<String>,
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
    // `agents` doesn't need a checkout — handle before resolution.
    if matches!(cli.command, Commands::Agents) {
        print!("{}", AGENTS_MD);
        return Ok(());
    }

    let checkout = checkout::resolve(
        cli.checkout_path.as_deref(),
        std::env::var("PERFTEST_BRAIN_CHECKOUT").ok(),
    )?;

    if cli.verbose > 0 {
        eprintln!("checkout: {} ({:?})", checkout.path.display(), checkout.vcs);
    }

    match cli.command {
        Commands::Info { input } => cmd_info(input.as_deref(), cli.json),
        Commands::Diagnose { input } => cmd_diagnose(input.as_deref(), cli.json, cli.verbose),
        Commands::Patch { input } => cmd_patch(input.as_deref(), &checkout, cli.json, cli.verbose),
        Commands::Sheriff { input } => cmd_sheriff(input.as_deref(), cli.json, cli.verbose),
        Commands::Groom => cmd_groom(cli.json, cli.verbose),
        Commands::Doctor { harness } => {
            cmd_doctor(harness.as_deref(), &checkout, cli.json, cli.verbose)
        }
        Commands::Update => cmd_update(&checkout, cli.json, cli.verbose),
        Commands::Agents => unreachable!("handled above"),
    }
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

    let url = "https://treeherder.mozilla.org/api/performance/alertsummary/?status=0&framework=13&limit=20";
    if verbose > 0 {
        eprintln!("Fetching untriaged browsertime alerts...");
    }

    let list: AlertList = get_json(url)?;
    let alert_ids: Vec<u64> = list
        .results
        .iter()
        .filter_map(|v| v.get("id")?.as_u64())
        .collect();

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
            "{:<8} {:<8} {:<12} {:<10} {}",
            "Alert", "Tier", "Framework", "Score", "Test"
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

fn cmd_update(checkout: &checkout::CheckoutRoot, json: bool, verbose: u8) -> anyhow::Result<()> {
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

fn cmd_diagnose(raw_input: Option<&str>, json: bool, verbose: u8) -> anyhow::Result<()> {
    let raw = raw_input.ok_or_else(|| {
        anyhow::anyhow!("Usage: perftest-brain diagnose <alert-id | URL | revision>")
    })?;

    let spec = input::parse_input(raw)?;
    let diag = diagnosis::diagnose(&spec, verbose > 0)?;

    if json {
        println!("{}", serde_json::to_string_pretty(&diag)?);
    } else {
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

    Ok(())
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
            if json {
                println!("{}", serde_json::to_string_pretty(&bug)?);
            } else {
                println!("Bug {}: {}", bug.id, bug.summary);
                println!("Status: {} {}", bug.status, bug.resolution);
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
