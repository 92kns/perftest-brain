use std::process::Command;
use std::time::Duration;

/// Timeout for subprocess execution. Enforcement is deferred to a later phase;
/// this const documents the intended limit. TODO(phase-later): kill subprocess after this.
pub const SUBPROCESS_TIMEOUT_SECS: u64 = 30;

/// Output captured from a subprocess invocation.
#[derive(Debug, Clone)]
pub struct ProcessOutput {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
}

/// Errors from subprocess execution.
#[derive(Debug, thiserror::Error)]
pub enum ToolError {
    #[error("tool not found on PATH: {0}")]
    NotFound(String),
    #[error("tool {name} exited {exit_code}: {stderr}")]
    Failed {
        name: String,
        exit_code: i32,
        stderr: String,
    },
    #[allow(dead_code)] // constructed when subprocess timeout enforcement is added
    #[error("tool {0} timed out after {1}s")]
    Timeout(String, u64),
}

/// A callable external CLI tool.
pub trait Tool {
    /// Name of the binary on PATH.
    fn name(&self) -> &str;

    /// Returns true if the binary is available on PATH.
    fn check_available(&self) -> bool {
        which_available(self.name())
    }

    /// Run the tool with `args`, capturing stdout + stderr.
    fn run(&self, args: &[&str]) -> Result<ProcessOutput, ToolError> {
        if !self.check_available() {
            return Err(ToolError::NotFound(self.name().to_owned()));
        }
        run_command(self.name(), args)
    }
}

/// Check if a binary named `name` exists on PATH by probing with `which`/`where`.
fn which_available(name: &str) -> bool {
    // Use std::process::Command to probe — avoid shell injection by never using sh -c.
    #[cfg(unix)]
    let check = Command::new("which").arg(name).output();
    #[cfg(windows)]
    let check = Command::new("where").arg(name).output();

    check.map(|o| o.status.success()).unwrap_or(false)
}

/// Execute `binary` with `args` using argv-style invocation (no shell).
fn run_command(binary: &str, args: &[&str]) -> Result<ProcessOutput, ToolError> {
    let output = Command::new(binary).args(args).output().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            ToolError::NotFound(binary.to_owned())
        } else {
            ToolError::Failed {
                name: binary.to_owned(),
                exit_code: -1,
                stderr: e.to_string(),
            }
        }
    })?;

    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let exit_code = output.status.code().unwrap_or(-1);

    Ok(ProcessOutput {
        stdout,
        stderr,
        exit_code,
    })
}

/// Generic implementation of `Tool` for any named CLI binary.
///
/// Ships in Phase 1 so the trait is exercised by tests. Later phases add
/// domain-specific wrappers (SearchfoxTool, StmoTool, PerfAlertTool, etc.).
pub struct CliTool {
    name: String,
}

impl CliTool {
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }
}

impl Tool for CliTool {
    fn name(&self) -> &str {
        &self.name
    }
}

/// Unused in Phase 1 — referenced only in tests and future phases.
#[allow(dead_code)]
pub(crate) fn _timeout_duration() -> Duration {
    Duration::from_secs(SUBPROCESS_TIMEOUT_SECS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_tool_runs_echo() {
        let tool = CliTool::new("echo");
        assert!(tool.check_available(), "echo should be on PATH");
        let out = tool.run(&["hello", "world"]).unwrap();
        assert!(out.stdout.contains("hello"), "stdout: {}", out.stdout);
        assert_eq!(out.exit_code, 0);
    }

    #[test]
    fn missing_tool_returns_not_found() {
        let tool = CliTool::new("__perftest_brain_nonexistent_tool__");
        assert!(!tool.check_available());
        let err = tool.run(&[]).unwrap_err();
        assert!(
            matches!(err, ToolError::NotFound(_)),
            "expected NotFound, got: {err}"
        );
        assert!(err
            .to_string()
            .contains("__perftest_brain_nonexistent_tool__"));
    }

    #[test]
    fn exit_code_is_captured() {
        // `false` is a standard Unix binary that exits 1.
        let tool = CliTool::new("false");
        if tool.check_available() {
            let out = tool.run(&[]).unwrap();
            assert_ne!(out.exit_code, 0, "false should exit non-zero");
        }
    }
}
