use anyhow::{bail, Result};
use std::path::{Path, PathBuf};

/// Version-control system backing a checkout. `jj` colocates with `.git`, so
/// the `.git` marker already resolves it — no `Jj` variant needed in Phase 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Vcs {
    Hg,
    Git,
}

/// A resolved Firefox checkout root and the VCS marker found there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckoutRoot {
    pub path: PathBuf,
    pub vcs: Vcs,
}

/// Resolve the Firefox checkout root.
///
/// Resolution order: explicit `flag` value → non-empty `env_var` value →
/// walk `std::env::current_dir()` upward. Returns an error if no checkout is
/// found, or if an explicit flag/env path is not a valid checkout.
pub fn resolve(flag: Option<&Path>, env_var: Option<String>) -> Result<CheckoutRoot> {
    if let Some(p) = flag {
        return validate(p);
    }
    if let Some(v) = env_var.filter(|s| !s.is_empty()) {
        return validate(Path::new(&v));
    }
    resolve_from(&std::env::current_dir()?)
}

/// Walk `start` upward, returning the innermost ancestor that is a valid
/// checkout. Factored out so the walk-up is testable without the real CWD.
fn resolve_from(start: &Path) -> Result<CheckoutRoot> {
    let mut dir = start.to_path_buf();
    loop {
        if let Some(root) = check_dir(&dir) {
            return Ok(root);
        }
        if !dir.pop() {
            bail!(
                "Not in a Firefox checkout. Run perftest-brain from inside \
                 mozilla-central, or pass --checkout-path <path>"
            );
        }
    }
}

/// Return a `CheckoutRoot` iff `dir` contains a `mach` file AND a `.hg` or
/// `.git` directory. The `mach` requirement is the two-marker guard against
/// over-broad detection (any git repo would otherwise match).
fn check_dir(dir: &Path) -> Option<CheckoutRoot> {
    if !dir.join("mach").is_file() {
        return None;
    }
    if dir.join(".hg").exists() {
        Some(CheckoutRoot {
            path: dir.to_path_buf(),
            vcs: Vcs::Hg,
        })
    } else if dir.join(".git").exists() {
        Some(CheckoutRoot {
            path: dir.to_path_buf(),
            vcs: Vcs::Git,
        })
    } else {
        None
    }
}

/// Validate an explicitly-supplied path (from `--checkout-path` or env override).
fn validate(p: &Path) -> Result<CheckoutRoot> {
    check_dir(p).ok_or_else(|| {
        anyhow::anyhow!(
            "{} is not a Firefox checkout (need a `mach` file with a `.hg` or `.git` sibling)",
            p.display()
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;
    use tempfile::tempdir;

    fn make_checkout(dir: &Path, vcs_marker: &str) {
        fs::write(dir.join("mach"), b"#!/usr/bin/env python3\n").unwrap();
        fs::create_dir_all(dir.join(vcs_marker)).unwrap();
    }

    #[test]
    fn flag_with_mach_and_git_resolves() {
        let tmp = tempdir().unwrap();
        make_checkout(tmp.path(), ".git");
        let root = resolve(Some(tmp.path()), None).unwrap();
        assert_eq!(root.path, tmp.path());
        assert_eq!(root.vcs, Vcs::Git);
    }

    #[test]
    fn flag_with_mach_and_hg_resolves() {
        let tmp = tempdir().unwrap();
        make_checkout(tmp.path(), ".hg");
        let root = resolve(Some(tmp.path()), None).unwrap();
        assert_eq!(root.vcs, Vcs::Hg);
    }

    #[test]
    fn git_without_mach_is_rejected() {
        let tmp = tempdir().unwrap();
        fs::create_dir_all(tmp.path().join(".git")).unwrap();
        let err = resolve(Some(tmp.path()), None).unwrap_err();
        assert!(err.to_string().contains("is not a Firefox checkout"));
    }

    #[test]
    fn env_var_resolves_valid_checkout() {
        let tmp = tempdir().unwrap();
        make_checkout(tmp.path(), ".git");
        let env = Some(tmp.path().to_string_lossy().into_owned());
        let root = resolve(None, env).unwrap();
        assert_eq!(root.path, tmp.path());
    }

    #[test]
    fn flag_wins_over_env() {
        let flag_dir = tempdir().unwrap();
        make_checkout(flag_dir.path(), ".hg");
        let env_dir = tempdir().unwrap();
        let env = Some(env_dir.path().to_string_lossy().into_owned());
        let root = resolve(Some(flag_dir.path()), env).unwrap();
        assert_eq!(root.path, flag_dir.path());
        assert_eq!(root.vcs, Vcs::Hg);
    }

    #[test]
    fn empty_env_is_ignored() {
        let tmp = tempdir().unwrap();
        // empty env string → falls through to walk-up, which fails in a bare tempdir
        let err = resolve(None, Some(String::new()));
        // should not error saying the empty string is not a checkout
        if let Err(e) = err {
            assert!(
                !e.to_string().contains("is not a Firefox checkout"),
                "empty env was incorrectly validated as a path: {e}"
            );
        }
    }

    #[test]
    fn walk_up_from_nested_subdir() {
        let tmp = tempdir().unwrap();
        make_checkout(tmp.path(), ".git");
        let nested = tmp.path().join("testing").join("raptor").join("tests");
        fs::create_dir_all(&nested).unwrap();
        let root = resolve_from(&nested).unwrap();
        assert_eq!(root.path, tmp.path());
    }

    #[test]
    fn walk_up_innermost_wins() {
        let outer = tempdir().unwrap();
        make_checkout(outer.path(), ".hg");
        let inner = outer.path().join("vendor").join("inner");
        fs::create_dir_all(&inner).unwrap();
        make_checkout(&inner, ".git");
        let start = inner.join("subdir");
        fs::create_dir_all(&start).unwrap();
        let root = resolve_from(&start).unwrap();
        assert_eq!(root.path, inner);
        assert_eq!(root.vcs, Vcs::Git);
    }

    #[test]
    fn no_checkout_gives_exact_error_message() {
        let tmp = tempdir().unwrap();
        let nested = tmp.path().join("a").join("b");
        fs::create_dir_all(&nested).unwrap();
        let err = resolve_from(&nested).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("Not in a Firefox checkout"), "msg: {msg}");
        assert!(msg.contains("--checkout-path"), "msg: {msg}");
    }
}
