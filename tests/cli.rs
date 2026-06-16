use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use tempfile::tempdir;

fn cmd() -> Command {
    Command::cargo_bin("perftest-brain").unwrap()
}

/// Create a fake Firefox checkout in `dir`.
fn make_checkout(dir: &std::path::Path) {
    fs::write(dir.join("mach"), b"#!/usr/bin/env python3\n").unwrap();
    fs::create_dir_all(dir.join(".git")).unwrap();
}

#[test]
fn help_succeeds() {
    cmd().arg("--help").assert().success();
}

#[test]
fn version_succeeds() {
    cmd().arg("--version").assert().success();
}

#[test]
fn outside_checkout_exits_nonzero() {
    let tmp = tempdir().unwrap();
    cmd()
        .current_dir(tmp.path())
        .env_remove("PERFTEST_BRAIN_CHECKOUT")
        .arg("info")
        .assert()
        .failure()
        .stderr(predicate::str::contains("Not in a Firefox checkout"));
}

#[test]
fn outside_checkout_json_flag_emits_json_error() {
    let tmp = tempdir().unwrap();
    cmd()
        .current_dir(tmp.path())
        .env_remove("PERFTEST_BRAIN_CHECKOUT")
        .args(["--json", "info"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("\"error\""))
        .stderr(predicate::str::contains("\"exit_code\""));
}

#[test]
fn checkout_path_flag_accepted() {
    let tmp = tempdir().unwrap();
    make_checkout(tmp.path());
    // info without arg → usage error (not a checkout-detection error)
    cmd()
        .args(["--checkout-path", tmp.path().to_str().unwrap(), "info"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Usage:").or(predicate::str::contains("info")));
}

#[test]
fn stub_subcommand_not_implemented() {
    let tmp = tempdir().unwrap();
    make_checkout(tmp.path());
    // All subcommands are now implemented — verify diagnose/sheriff/doctor require args
    for sub in &["diagnose", "sheriff", "doctor"] {
        cmd()
            .args(["--checkout-path", tmp.path().to_str().unwrap(), sub])
            .assert()
            .failure()
            .stderr(predicate::str::contains("Usage:").or(predicate::str::contains("error:")));
    }
}
