use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::tools::{CliTool, Tool};

/// A diagnostic check result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckResult {
    pub name: String,
    pub status: CheckStatus,
    pub detail: String,
    pub fix_hint: Option<String>,
}

/// Whether a check passed, warned, or failed.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum CheckStatus {
    Ok,
    Warn,
    Fail,
}

impl std::fmt::Display for CheckStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CheckStatus::Ok => write!(f, "OK"),
            CheckStatus::Warn => write!(f, "WARN"),
            CheckStatus::Fail => write!(f, "FAIL"),
        }
    }
}

/// Tool doctor report. [TDOC-01, TDOC-02]
#[derive(Debug, Serialize, Deserialize)]
pub struct DoctorReport {
    pub harness: String,
    pub checks: Vec<CheckResult>,
    pub overall: CheckStatus,
}

/// Run the doctor for the specified harness. [TDOC-01, TDOC-02]
pub fn run_doctor(harness: &str, checkout_root: &Path, verbose: bool) -> Result<DoctorReport> {
    let harness_lower = harness.to_lowercase();
    let checks = match harness_lower.as_str() {
        "raptor" => check_raptor(checkout_root, verbose),
        "mozperftest" => check_mozperftest(checkout_root, verbose),
        other => {
            return Ok(DoctorReport {
                harness: other.to_owned(),
                checks: vec![CheckResult {
                    name: "harness".into(),
                    status: CheckStatus::Fail,
                    detail: format!(
                        "Unknown harness: {:?}. Supported: raptor, mozperftest",
                        other
                    ),
                    fix_hint: Some(
                        "Run: perftest-brain doctor raptor  OR  perftest-brain doctor mozperftest"
                            .into(),
                    ),
                }],
                overall: CheckStatus::Fail,
            });
        }
    };

    let overall = if checks.iter().any(|c| c.status == CheckStatus::Fail) {
        CheckStatus::Fail
    } else if checks.iter().any(|c| c.status == CheckStatus::Warn) {
        CheckStatus::Warn
    } else {
        CheckStatus::Ok
    };

    Ok(DoctorReport {
        harness: harness.to_owned(),
        checks,
        overall,
    })
}

fn check_raptor(checkout_root: &Path, verbose: bool) -> Vec<CheckResult> {
    let mut checks = Vec::new();

    // Check raptor directory exists
    checks.push(check_dir_exists(
        "raptor directory",
        &checkout_root.join("testing/raptor"),
        "Clone mozilla-central and ensure testing/raptor/ is present.",
    ));

    // Check Python is available
    checks.push(check_binary(
        "python3",
        "python3",
        "--version",
        "Install Python 3.x: https://www.python.org/downloads/",
    ));

    // Check mach is present
    checks.push(check_file_exists(
        "mach script",
        &checkout_root.join("mach"),
        "Run from inside a mozilla-central checkout.",
    ));

    // Check raptor requirements file
    checks.push(check_file_exists(
        "raptor requirements",
        &checkout_root.join("testing/raptor/requirements.txt"),
        "Raptor requirements file missing. Try: cd testing/raptor && pip install -r requirements.txt",
    ));

    // Check geckodriver
    checks.push(check_binary(
        "geckodriver",
        "geckodriver",
        "--version",
        "Install geckodriver: https://github.com/mozilla/geckodriver/releases",
    ));

    // Check node (for browsertime)
    checks.push(check_binary(
        "node.js",
        "node",
        "--version",
        "Install Node.js >= 18: https://nodejs.org/",
    ));

    if verbose {
        eprintln!("Raptor checks complete: {}/{} OK",
            checks.iter().filter(|c| c.status == CheckStatus::Ok).count(),
            checks.len()
        );
    }

    checks
}

fn check_mozperftest(checkout_root: &Path, verbose: bool) -> Vec<CheckResult> {
    let mut checks = Vec::new();

    // Check mozperftest directories
    checks.push(check_dir_exists(
        "mozperftest directory",
        &checkout_root.join("testing/mozperftest"),
        "Clone mozilla-central and ensure testing/mozperftest/ is present.",
    ));

    checks.push(check_dir_exists(
        "performance directory",
        &checkout_root.join("testing/performance"),
        "Clone mozilla-central and ensure testing/performance/ is present.",
    ));

    // Check Python
    checks.push(check_binary(
        "python3",
        "python3",
        "--version",
        "Install Python 3.x: https://www.python.org/downloads/",
    ));

    // Check mach
    checks.push(check_file_exists(
        "mach script",
        &checkout_root.join("mach"),
        "Run from inside a mozilla-central checkout.",
    ));

    // Check mozperftest runner exists
    checks.push(check_file_exists(
        "mozperftest runner",
        &checkout_root.join("testing/mozperftest/runner.py"),
        "runner.py missing. Check your checkout is up to date.",
    ));

    // Check node (mozperftest also uses browsertime)
    checks.push(check_binary(
        "node.js",
        "node",
        "--version",
        "Install Node.js >= 18: https://nodejs.org/",
    ));

    if verbose {
        eprintln!("mozperftest checks complete: {}/{} OK",
            checks.iter().filter(|c| c.status == CheckStatus::Ok).count(),
            checks.len()
        );
    }

    checks
}

fn check_dir_exists(name: &str, path: &Path, fix_hint: &str) -> CheckResult {
    if path.is_dir() {
        CheckResult {
            name: name.to_owned(),
            status: CheckStatus::Ok,
            detail: format!("Found: {}", path.display()),
            fix_hint: None,
        }
    } else {
        CheckResult {
            name: name.to_owned(),
            status: CheckStatus::Fail,
            detail: format!("Not found: {}", path.display()),
            fix_hint: Some(fix_hint.to_owned()),
        }
    }
}

fn check_file_exists(name: &str, path: &Path, fix_hint: &str) -> CheckResult {
    if path.is_file() {
        CheckResult {
            name: name.to_owned(),
            status: CheckStatus::Ok,
            detail: format!("Found: {}", path.display()),
            fix_hint: None,
        }
    } else {
        CheckResult {
            name: name.to_owned(),
            status: CheckStatus::Fail,
            detail: format!("Not found: {}", path.display()),
            fix_hint: Some(fix_hint.to_owned()),
        }
    }
}

fn check_binary(name: &str, binary: &str, version_arg: &str, fix_hint: &str) -> CheckResult {
    let tool = CliTool::new(binary);
    if !tool.check_available() {
        return CheckResult {
            name: name.to_owned(),
            status: CheckStatus::Fail,
            detail: format!("{binary} not found on PATH"),
            fix_hint: Some(fix_hint.to_owned()),
        };
    }

    match tool.run(&[version_arg]) {
        Ok(out) if out.exit_code == 0 => CheckResult {
            name: name.to_owned(),
            status: CheckStatus::Ok,
            detail: out.stdout.lines().next().unwrap_or("available").trim().to_owned(),
            fix_hint: None,
        },
        Ok(out) => CheckResult {
            name: name.to_owned(),
            status: CheckStatus::Warn,
            detail: format!("{binary} found but version check failed: {}", out.stderr.trim()),
            fix_hint: Some(fix_hint.to_owned()),
        },
        Err(e) => CheckResult {
            name: name.to_owned(),
            status: CheckStatus::Warn,
            detail: format!("{binary} found but errored: {e}"),
            fix_hint: Some(fix_hint.to_owned()),
        },
    }
}
