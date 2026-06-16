use anyhow::{bail, Result};
use std::path::Path;

use crate::checkout::{CheckoutRoot, Vcs};
use crate::tools::{CliTool, Tool};

/// VCS cleanliness check result.
#[derive(Debug)]
pub enum VcsState {
    Clean,
    Dirty { details: String },
    Unknown,
}

/// Detect whether the working copy is clean before patching. [PATCH-02]
///
/// Refuses to patch a dirty checkout with a clear message.
pub fn check_working_copy(checkout: &CheckoutRoot) -> Result<VcsState> {
    match checkout.vcs {
        Vcs::Git => check_git(&checkout.path),
        Vcs::Hg => check_hg(&checkout.path),
    }
}

fn check_git(root: &Path) -> Result<VcsState> {
    let tool = CliTool::new("git");
    if !tool.check_available() {
        return Ok(VcsState::Unknown);
    }

    let out = tool.run(&["-C", root.to_str().unwrap_or("."), "status", "--porcelain"])?;

    if out.exit_code != 0 {
        bail!(
            "git status failed (exit {}): {}",
            out.exit_code,
            out.stderr.trim()
        );
    }

    let dirty = out.stdout.trim();
    if dirty.is_empty() {
        Ok(VcsState::Clean)
    } else {
        Ok(VcsState::Dirty {
            details: dirty.to_owned(),
        })
    }
}

fn check_hg(root: &Path) -> Result<VcsState> {
    let tool = CliTool::new("hg");
    if !tool.check_available() {
        // Try jj as an alternative
        return check_jj(root);
    }

    let out = tool.run(&["-R", root.to_str().unwrap_or("."), "status"])?;

    if out.exit_code != 0 {
        bail!(
            "hg status failed (exit {}): {}",
            out.exit_code,
            out.stderr.trim()
        );
    }

    let dirty = out.stdout.trim();
    if dirty.is_empty() {
        Ok(VcsState::Clean)
    } else {
        Ok(VcsState::Dirty {
            details: dirty.to_owned(),
        })
    }
}

fn check_jj(root: &Path) -> Result<VcsState> {
    let tool = CliTool::new("jj");
    if !tool.check_available() {
        return Ok(VcsState::Unknown);
    }

    let out = tool.run(&["--repository", root.to_str().unwrap_or("."), "status"])?;

    if out.exit_code != 0 {
        return Ok(VcsState::Unknown);
    }

    // jj outputs "Working copy changes:" followed by changed files
    let has_changes =
        out.stdout.contains("Working copy changes:") && out.stdout.lines().count() > 1;

    if has_changes {
        Ok(VcsState::Dirty {
            details: out.stdout.trim().to_owned(),
        })
    } else {
        Ok(VcsState::Clean)
    }
}

/// Ensure the checkout is clean before patching. Returns `Err` with a clear
/// message if dirty. [PATCH-02]
pub fn assert_clean(checkout: &CheckoutRoot) -> Result<()> {
    match check_working_copy(checkout)? {
        VcsState::Clean => Ok(()),
        VcsState::Unknown => {
            // Can't verify — warn but don't block
            eprintln!(
                "Warning: could not verify VCS cleanliness (VCS tool not found). \
                 Proceeding — ensure your working copy is clean before applying patches."
            );
            Ok(())
        }
        VcsState::Dirty { details } => {
            bail!(
                "Working copy is not clean. Refusing to patch to avoid losing uncommitted changes.\n\
                 \n\
                 Modified files:\n{}\n\
                 \n\
                 Commit or stash your changes, then re-run perftest-brain patch.",
                details
            )
        }
    }
}
