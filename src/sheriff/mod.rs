use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::api;
use crate::types::{AlertSummary, InputSpec};

/// Sheriff tier classification. [SHRFF-01]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum Tier {
    /// Critical — immediate attention, backout likely required.
    Tier1,
    /// Important — investigate within 24 hours.
    Tier2,
    /// Low priority — monitor, likely noise or minor.
    Tier3,
}

impl std::fmt::Display for Tier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Tier::Tier1 => write!(f, "Tier 1"),
            Tier::Tier2 => write!(f, "Tier 2"),
            Tier::Tier3 => write!(f, "Tier 3"),
        }
    }
}

/// Sheriff analysis result. [SHRFF-01, SHRFF-02]
#[derive(Debug, Serialize, Deserialize)]
pub struct SheriffAnalysis {
    pub alert_id: Option<u64>,
    pub tier: Tier,
    pub backout_recommended: bool,
    pub backout_reasoning: String,
    pub classification_reasoning: String,
    pub framework: String,
    pub test_summary: String,
    pub worst_regression_pct: f64,
    pub affected_platforms: Vec<String>,
}

/// Classify an alert and produce a sheriff analysis. [SHRFF-01, SHRFF-02]
pub fn analyze(spec: &InputSpec, verbose: bool) -> Result<SheriffAnalysis> {
    match spec {
        InputSpec::Alert { alert_id } => {
            if verbose {
                eprintln!("Fetching alert {} for sheriff analysis...", alert_id);
            }
            let summary = api::perfherder::fetch_alert_summary(*alert_id)?;
            Ok(classify_alert(*alert_id, &summary))
        }
        _ => Ok(SheriffAnalysis {
            alert_id: None,
            tier: Tier::Tier3,
            backout_recommended: false,
            backout_reasoning: "Input is not a direct alert ID — resolve to an alert first.".into(),
            classification_reasoning: "Cannot classify without an alert summary.".into(),
            framework: "unknown".into(),
            test_summary: format!("{:?}", spec),
            worst_regression_pct: 0.0,
            affected_platforms: vec![],
        }),
    }
}

fn classify_alert(alert_id: u64, summary: &AlertSummary) -> SheriffAnalysis {
    let worst_pct = summary
        .regressions
        .iter()
        .map(|r| r.delta_percent.abs())
        .fold(0.0_f64, f64::max);

    let affected_platforms: Vec<String> = {
        let mut seen = std::collections::HashSet::new();
        summary
            .regressions
            .iter()
            .filter_map(|r| {
                if seen.insert(r.platform.clone()) {
                    Some(r.platform.clone())
                } else {
                    None
                }
            })
            .collect()
    };

    let is_critical_framework = matches!(
        summary.framework.as_str(),
        "browsertime" | "talos" | "awsy" | "mozperftest"
    );

    let is_primary_test = summary.regressions.iter().any(|r| {
        let t = r.test.to_lowercase();
        t.contains("speedometer")
            || t.contains("tp6")
            || t.contains("startup")
            || t.contains("memory")
    });

    let multi_platform = affected_platforms.len() > 1;

    // Tier classification logic
    let (tier, class_reason) = if worst_pct >= 10.0 && is_critical_framework && (is_primary_test || multi_platform) {
        (
            Tier::Tier1,
            format!(
                "{}% regression on {} primary test(s), {} platform(s) — exceeds 10% threshold on critical framework",
                worst_pct as u32, summary.framework, affected_platforms.len()
            ),
        )
    } else if worst_pct >= 5.0 && is_critical_framework {
        (
            Tier::Tier2,
            format!(
                "{}% regression on {} — above 5% threshold",
                worst_pct as u32, summary.framework
            ),
        )
    } else if worst_pct >= 2.0 || !summary.regressions.is_empty() {
        (
            Tier::Tier2,
            format!(
                "{}% regression detected — investigate within 24h",
                worst_pct as u32
            ),
        )
    } else {
        (
            Tier::Tier3,
            "Small regression or improvement only — monitor but no immediate action needed.".into(),
        )
    };

    // Backout recommendation
    let (backout_recommended, backout_reasoning) = if tier == Tier::Tier1 {
        (
            true,
            format!(
                "Tier 1 — recommend backout: {:.1}% regression on {} {} ({})",
                worst_pct,
                summary.framework,
                summary
                    .regressions
                    .first()
                    .map(|r| format!("{}/{}", r.suite, r.test))
                    .unwrap_or_default(),
                affected_platforms.join(", ")
            ),
        )
    } else if tier == Tier::Tier2 && multi_platform {
        (
            true,
            format!(
                "Tier 2 multi-platform regression ({:.1}% on {}) — backout recommended",
                worst_pct, summary.framework
            ),
        )
    } else {
        (
            false,
            format!(
                "Tier {} — investigation recommended before backout decision",
                if tier == Tier::Tier2 { "2" } else { "3" }
            ),
        )
    };

    let test_summary = summary
        .regressions
        .first()
        .map(|r| format!("{}/{} on {}", r.suite, r.test, r.platform))
        .unwrap_or_else(|| summary.improvements.first().map(|r| format!("improvement: {}/{}", r.suite, r.test)).unwrap_or_else(|| "no alerts".into()));

    SheriffAnalysis {
        alert_id: Some(alert_id),
        tier,
        backout_recommended,
        backout_reasoning,
        classification_reasoning: class_reason,
        framework: summary.framework.clone(),
        test_summary,
        worst_regression_pct: worst_pct,
        affected_platforms,
    }
}

/// Alert entry for grooming. [SHRFF-03]
#[derive(Debug, Serialize, Deserialize)]
pub struct GroomEntry {
    pub alert_id: u64,
    pub tier: Tier,
    pub score: f64,
    pub framework: String,
    pub test_summary: String,
    pub status: String,
    pub suggested_owner: Option<String>,
    pub treeherder_url: String,
}

/// Groom (rank) a list of alert IDs. [SHRFF-03]
pub fn groom(alert_ids: &[u64], verbose: bool) -> Result<Vec<GroomEntry>> {
    let mut entries = Vec::new();

    for &id in alert_ids {
        if verbose {
            eprintln!("Fetching alert {} for grooming...", id);
        }
        let summary = match api::perfherder::fetch_alert_summary(id) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("Warning: could not fetch alert {id}: {e}");
                continue;
            }
        };

        let analysis = classify_alert(id, &summary);
        let tier_score = match analysis.tier {
            Tier::Tier1 => 100.0,
            Tier::Tier2 => 50.0,
            Tier::Tier3 => 10.0,
        };
        let score = tier_score + analysis.worst_regression_pct;

        let suggested_owner = infer_owner(&summary);

        entries.push(GroomEntry {
            alert_id: id,
            tier: analysis.tier,
            score,
            framework: summary.framework.clone(),
            test_summary: analysis.test_summary,
            status: summary.status,
            suggested_owner,
            treeherder_url: format!(
                "https://treeherder.mozilla.org/perfherder/alerts?id={id}"
            ),
        });
    }

    // Sort by score descending (highest priority first)
    entries.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    Ok(entries)
}

fn infer_owner(summary: &AlertSummary) -> Option<String> {
    // Suggest owner based on framework
    Some(match summary.framework.as_str() {
        "browsertime" | "raptor" => "perf-sheriffs@mozilla.com".into(),
        "awsy" => "memsheriffs@mozilla.com".into(),
        "talos" => "talos-sheriff@mozilla.com".into(),
        _ => "perf-sheriffs@mozilla.com".into(),
    })
}
