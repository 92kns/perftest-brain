pub mod manifest;
pub mod vcs;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::checkout::CheckoutRoot;
use crate::diagnosis::{Diagnosis, SignalType};
use crate::types::InputSpec;

/// The result of a patch operation.
#[derive(Debug, Serialize, Deserialize)]
pub struct PatchResult {
    pub applied: Vec<PatchAction>,
    pub skipped: Vec<String>,
    pub next_steps: Vec<String>,
}

/// A single patch action that was applied.
#[derive(Debug, Serialize, Deserialize)]
pub struct PatchAction {
    pub file: String,
    pub description: String,
}

/// Patch an intermittent from an `InputSpec`.
///
/// Flow: diagnose → VCS clean check → determine fix → apply → show diff. [PATCH-01..04]
pub fn patch(
    spec: &InputSpec,
    checkout: &CheckoutRoot,
    auto_apply: bool,
    verbose: bool,
) -> Result<PatchResult> {
    // 1. Diagnose first to understand what needs fixing
    let diag = crate::diagnosis::diagnose(spec, verbose)?;

    if diag.confidence == crate::diagnosis::Confidence::Insufficient {
        return Ok(PatchResult {
            applied: vec![],
            skipped: vec![diag.input_summary.clone()],
            next_steps: diag.next_steps,
        });
    }

    // 2. VCS clean check [PATCH-02]
    vcs::assert_clean(checkout)?;

    // 3. Determine fixes from diagnosis findings
    let fixes = determine_fixes(&diag, checkout);

    if fixes.is_empty() {
        return Ok(PatchResult {
            applied: vec![],
            skipped: vec![
                "No patchable issues found — diagnosis did not match any fix template.".into(),
            ],
            next_steps: diag.next_steps,
        });
    }

    let mut applied = Vec::new();
    let mut skipped = Vec::new();

    for fix in &fixes {
        // 4. Show diff preview unless --yes [PATCH-03]
        if !auto_apply {
            eprintln!("Would apply: {}", fix.description);
            eprintln!("  File: {}", fix.target_file);
            // In non-auto mode, we still apply (interactive confirm happens in main)
        }

        // 5. Apply fix atomically [PATCH-04]
        match manifest::apply_fix(fix, &checkout.path) {
            Ok(msg) => {
                applied.push(PatchAction {
                    file: fix.target_file.clone(),
                    description: msg,
                });
            }
            Err(e) => {
                skipped.push(format!("{}: {}", fix.target_file, e));
            }
        }
    }

    let mut next_steps = vec![
        "Review the changes with: hg diff / git diff".into(),
        "Run the test locally to verify the fix.".into(),
        "Submit via: moz-phab submit / git push + phabricator".into(),
    ];

    if matches!(diag.signal_type, SignalType::SustainedRegression) {
        next_steps.insert(
            0,
            "Note: This signal may be a sustained regression, not an intermittent. \
             Verify the fix is appropriate before submitting."
                .into(),
        );
    }

    Ok(PatchResult {
        applied,
        skipped,
        next_steps,
    })
}

/// Determine which manifest fixes to apply based on diagnosis findings.
fn determine_fixes(diag: &Diagnosis, checkout: &CheckoutRoot) -> Vec<manifest::ManifestFix> {
    let mut fixes = Vec::new();

    for finding in &diag.findings {
        match finding.category.as_str() {
            "timeout" => {
                // Find the test manifest file and add requestLongerTimeout
                if let Some(test_file) = find_test_manifest(&diag.input_summary, &checkout.path) {
                    fixes.push(manifest::ManifestFix::longer_timeout(test_file, 2));
                }
            }
            "no_data" | "infrastructure" => {
                // Infrastructure / transient — add a skip-if for the failing platform
                if let Some(platform) = extract_platform(&diag.input_summary) {
                    if let Some(test_file) = find_test_manifest(&diag.input_summary, &checkout.path)
                    {
                        fixes.push(manifest::ManifestFix::skip_if(
                            test_file,
                            &format!("os == '{platform}'"),
                            "intermittent failure on this platform",
                        ));
                    }
                }
            }
            _ => {}
        }
    }

    fixes
}

/// Search for the test manifest file corresponding to a test name.
///
/// In Phase 5 we do a basic search of the index. Later phases can refine this.
fn find_test_manifest(input_summary: &str, _checkout_root: &Path) -> Option<String> {
    // Extract test name hint from the input summary
    let words: Vec<&str> = input_summary.split_whitespace().collect();
    let test_hint = words
        .iter()
        .find(|w| w.contains('/') || w.ends_with(".toml") || w.ends_with(".ini"))?;

    if test_hint.ends_with(".toml") || test_hint.ends_with(".ini") {
        return Some(test_hint.to_string());
    }

    None
}

fn extract_platform(input_summary: &str) -> Option<String> {
    // Look for platform keywords in the summary
    for keyword in &["linux", "windows", "mac", "android"] {
        if input_summary.to_lowercase().contains(keyword) {
            return Some(keyword.to_string());
        }
    }
    None
}
