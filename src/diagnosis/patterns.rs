/// The type of fix typically needed for a pattern.
///
/// All variants are valid fix types. `SkipIf` and `CodeFix` are used in
/// pattern definitions and consumed by the patch engine.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FixType {
    /// Retrigger — likely transient infra, no code change needed.
    Retrigger,
    /// Add `requestLongerTimeout` to the test manifest.
    RequestLongerTimeout,
    /// Add a `skip-if` condition for the affected platform.
    SkipIf,
    /// Code fix required in test or harness.
    CodeFix,
    /// Report to infra — hardware or CI worker issue.
    InfraReport,
    /// File a browser crash bug separately.
    FileCrashBug,
}

/// A failure signature pattern — matches log text to a known failure category.
///
/// All patterns are grounded in real Bugzilla intermittent failure bugs and
/// live Taskcluster job logs from the Treeherder failures corpus.
pub struct Pattern {
    pub category: &'static str,
    pub description: &'static str,
    /// Substrings that must ALL appear in the log to match.
    pub matches: &'static [&'static str],
    pub root_cause: &'static str,
    pub next_step: &'static str,
    pub fix_type: FixType,
    /// Platforms where this pattern is most common. Empty = all platforms.
    pub platform_hints: &'static [&'static str],
    /// Representative Bugzilla bug, for context.
    pub example_bug: Option<u64>,
}

/// Known perf-test failure patterns grounded in the real Bugzilla corpus.
///
/// Sources:
/// - Treeherder failures API (autoland, 2026-05-01 to 2026-06-16)
/// - Bugzilla Testing/Raptor and Testing/mozperftest intermittent-failure bugs
/// - Live Taskcluster job logs from failed tasks
/// - Treeherder failure count data (top intermittents by frequency)
pub static PATTERNS: &[Pattern] = &[
    // ── Browsertime: browser won't start ─────────────────────────────────────
    Pattern {
        category: "browser_start",
        description: "Browsertime could not start the browser after 3 tries",
        matches: &["BrowserError: Could not start the browser"],
        root_cause: "Firefox failed to launch during browsertime run. Common causes: \
                     Android device ADB issue, hardware problem, or GeckoDriver mismatch.",
        next_step: "Retrigger the job. If it fails on Android devices consistently, \
                    run `perftest-brain doctor raptor` to check GeckoDriver and ADB. \
                    File an infra bug if it affects a specific device pool (Bug 1635752 pattern).",
        fix_type: FixType::Retrigger,
        platform_hints: &["android"],
        example_bug: Some(1635752),
    },

    // ── Browsertime: generic failure to run ──────────────────────────────────
    Pattern {
        category: "browsertime_failed",
        description: "Browsertime failed to run (generic runner failure)",
        matches: &["Browsertime failed to run"],
        root_cause: "The browsertime Node.js runner threw an unhandled error. \
                     May be a device connectivity issue (Android), a Node.js version problem, \
                     or a transient network issue during test page fetch.",
        next_step: "Retrigger. If persistent: run `perftest-brain doctor raptor` to check \
                    Node.js version and geckodriver. On Android, check ADB connection.",
        fix_type: FixType::Retrigger,
        platform_hints: &[],
        example_bug: Some(1638702),
    },

    // ── Browsertime: page load timeout ───────────────────────────────────────
    Pattern {
        category: "timeout",
        description: "Browsertime timed out waiting for page to load",
        matches: &["Failed waiting on page", "timed out after 300000 ms"],
        root_cause: "The test page didn't finish loading within browsertime's 300s limit. \
                     Causes: slow network on the worker, mitmproxy recording stale/slow, \
                     or the page itself regressed in load time.",
        next_step: "Add `requestLongerTimeout` to the test's raptor .toml manifest. \
                    Also check if mitmproxy recording needs a refresh.",
        fix_type: FixType::RequestLongerTimeout,
        platform_hints: &[],
        example_bug: Some(1641648),
    },

    // ── Raptor: page load timeout (older format) ─────────────────────────────
    Pattern {
        category: "timeout",
        description: "Raptor test timed out loading the test page",
        matches: &["timed out loading test page"],
        root_cause: "Raptor test exceeded page load timeout. Common for pageload (tp6) tests \
                     on slow network workers or when mitmproxy replay is slow. \
                     If this follows a mitmproxy version upgrade, the recording needs to be \
                     re-recorded with the new version (Bug 1513467: fix was 're-record for mitm4').",
        next_step: "First: retrigger to confirm intermittency. \
                    If it persists: add `requestLongerTimeout` to the raptor .toml manifest. \
                    If it started after a mitmproxy upgrade: re-record with `./mach raptor --record`. \
                    If it fails >30x/week: disable with `skip-if = true` (Bug 1513467 pattern).",
        fix_type: FixType::RequestLongerTimeout,
        platform_hints: &[],
        example_bug: Some(1513467),
    },

    // ── Mitmproxy: recording stale after version upgrade ─────────────────────
    Pattern {
        category: "mitmproxy",
        description: "Mitmproxy recording is stale or incompatible with current mitmproxy version",
        matches: &["mitmproxy", "timed out loading test page"],
        root_cause: "The mitmproxy recording was created with an older version of mitmproxy. \
                     After a mitmproxy version upgrade the replay may fail silently, causing \
                     page load timeouts. Real fix: re-record the test (Bug 1513467).",
        next_step: "Re-record the test with the current mitmproxy version: \
                    `./mach raptor --record --test <test-name>`. \
                    Check `testing/raptor/raptor/playback/mitmproxy*.py` for the current version.",
        fix_type: FixType::CodeFix,
        platform_hints: &[],
        example_bug: Some(1513467),
    },

    // ── Browsertime: NoSuchWindow after timeout ───────────────────────────────
    Pattern {
        category: "timeout",
        description: "Browsertime timed out and browser window no longer exists",
        matches: &["timed out after 300000 ms", "NoSuchWindow"],
        root_cause: "Page load timed out AND the browser window closed unexpectedly. \
                     This usually means Firefox crashed during the test.",
        next_step: "Look for a crash report earlier in the same log. \
                    File a crash bug if the crash reproduces. Add `skip-if` for the platform \
                    if the crash is known-intermittent.",
        fix_type: FixType::FileCrashBug,
        platform_hints: &[],
        example_bug: Some(1642045),
    },

    // ── Raptor/Browsertime: no test results ──────────────────────────────────
    Pattern {
        category: "no_data",
        description: "No raptor test results were found",
        matches: &["no raptor test results"],
        root_cause: "Test ran but produced no PERFHERDER_DATA output. \
                     Browser may have crashed before measurements were taken, \
                     or the test script failed to emit results.",
        next_step: "Look earlier in the log for a browser crash or Python exception. \
                    Retrigger once — if it fails consistently, \
                    the test script may have a bug.",
        fix_type: FixType::Retrigger,
        platform_hints: &[],
        example_bug: Some(1499253),
    },

    // ── Browsertime: missing metric measurements ──────────────────────────────
    Pattern {
        category: "no_data",
        description: "Browsertime cycle missing a required measurement (e.g. firstPaint)",
        matches: &["MissingResultsError", "Browsertime cycle missing"],
        root_cause: "The test ran but the page didn't expose the expected metric. \
                     Could be a page change, test timing issue, or metric API not available.",
        next_step: "Retrigger to confirm it's intermittent. If consistent, \
                    check if the page still exposes the metric. \
                    May need a test script update.",
        fix_type: FixType::Retrigger,
        platform_hints: &[],
        example_bug: Some(1651851),
    },

    // ── Browsertime: no measurements at all ──────────────────────────────────
    Pattern {
        category: "no_data",
        description: "Browsertime produced no measurements",
        matches: &["Browsertime produced no measurements"],
        root_cause: "Browsertime completed but returned zero measurement data. \
                     Likely a script execution failure or page not loading at all.",
        next_step: "Retrigger. If consistent, add debug logging to the browsertime script \
                    to find where measurement collection fails.",
        fix_type: FixType::Retrigger,
        platform_hints: &[],
        example_bug: Some(1585199),
    },

    // ── Mitmproxy: proxy failure ──────────────────────────────────────────────
    Pattern {
        category: "mitmproxy",
        description: "Mitmproxy recording playback failed with a traceback",
        matches: &["raptor-mitmproxy", "Traceback"],
        root_cause: "The mitmproxy recording replay failed. \
                     Possible causes: recording is stale, network change, \
                     or a Python/mitmproxy version issue.",
        next_step: "Retrigger. If persistent, check if the mitmproxy recording needs \
                    to be re-recorded for this test.",
        fix_type: FixType::Retrigger,
        platform_hints: &[],
        example_bug: Some(1509233),
    },

    // ── Taskcluster: max run time exceeded ────────────────────────────────────
    Pattern {
        category: "infrastructure",
        description: "Task aborted — max run time exceeded",
        matches: &["Task aborted - max run time exceeded"],
        root_cause: "The CI task hit its time limit. On Android devices this often means \
                     the hg/git clone got stuck (Bug 2038441). On desktop it can mean \
                     a test loop or slow worker.",
        next_step: "Retrigger. If it's an Android device consistently, \
                    check if hg cloning is working. \
                    File an infra bug if it's device-specific (Bug 1809667 pattern).",
        fix_type: FixType::InfraReport,
        platform_hints: &[],
        example_bug: Some(1809667),
    },

    // ── Taskcluster: artifact file missing on worker ──────────────────────────
    Pattern {
        category: "infrastructure",
        description: "Task aborted before artifacts were produced",
        matches: &["file-missing-on-worker", "Could not read"],
        root_cause: "The task was killed (likely max run time) before it produced \
                     any output artifacts. No test data was collected.",
        next_step: "Retrigger. The task didn't get far enough to run tests. \
                    This is usually an infrastructure issue, not a test bug.",
        fix_type: FixType::Retrigger,
        platform_hints: &[],
        example_bug: Some(2038441),
    },

    // ── Linux display: no pipewire socket ────────────────────────────────────
    Pattern {
        category: "infrastructure",
        description: "Linux worker: no pipewire display socket (worker will retry)",
        matches: &["error: no pipewire socket"],
        root_cause: "The Linux CI worker's display environment (pipewire) wasn't ready \
                     when the test started. The worker script retries automatically.",
        next_step: "Worker will retry this automatically. If the job still fails, \
                    retrigger manually. File an infra bug if the frequency is high on a \
                    specific worker pool.",
        fix_type: FixType::Retrigger,
        platform_hints: &["linux"],
        example_bug: None,
    },

    // ── Firefox crash: RunWatchdog ────────────────────────────────────────────
    Pattern {
        category: "browser_crash",
        description: "Firefox killed by RunWatchdog (too slow to shut down)",
        matches: &["RunWatchdog"],
        root_cause: "Firefox watchdog timer killed the process because shutdown took \
                     longer than allowed. Very high frequency meta-bug (Bug 1358898, 836+ failures/week). \
                     Often triggered during test cleanup.",
        next_step: "Retrigger — this is a known high-frequency intermittent (Bug 1358898). \
                    If it's blocking a specific test consistently, add `skip-if` for the platform. \
                    Don't file a new bug — add to the meta bug instead.",
        fix_type: FixType::Retrigger,
        platform_hints: &[],
        example_bug: Some(1358898),
    },

    // ── AWSY: Marionette session lost (Firefox crash) ─────────────────────────
    Pattern {
        category: "browser_crash",
        description: "AWSY: Marionette session lost — Firefox crashed during memory test",
        matches: &["InvalidSessionIdException", "awsy"],
        root_cause: "Firefox crashed during the AWSY memory test, causing the Marionette \
                     connection to become invalid. The test cannot continue without a live session.",
        next_step: "Retrigger. If Firefox crashes consistently during AWSY, \
                    file a browser crash bug with the minidump from the artifacts.",
        fix_type: FixType::FileCrashBug,
        platform_hints: &["linux", "windows"],
        example_bug: None,
    },

    // ── AWSY: test failure exit code ─────────────────────────────────────────
    Pattern {
        category: "browser_crash",
        description: "AWSY exited with failure (return code 10)",
        matches: &["AWSY exited with return code 10"],
        root_cause: "AWSY harness exited with its failure exit code. Usually accompanies \
                     a crash or Marionette session loss.",
        next_step: "Look earlier in the log for the specific failure (crash, \
                    InvalidSessionIdException, or unexpected status). Retrigger.",
        fix_type: FixType::Retrigger,
        platform_hints: &[],
        example_bug: None,
    },

    // ── Browsertime: Marionette decode error ─────────────────────────────────
    Pattern {
        category: "browser_crash",
        description: "WebDriverError: Failed to decode Marionette response",
        matches: &["WebDriverError: Failed to decode response from marionette"],
        root_cause: "Firefox crashed or hung, causing the Marionette protocol \
                     to return an undecodable response.",
        next_step: "Retrigger. Look for a crash minidump in the Taskcluster artifacts. \
                    File a crash bug if the crash reproduces.",
        fix_type: FixType::FileCrashBug,
        platform_hints: &[],
        example_bug: Some(1642205),
    },

    // ── Android ADB timeout ───────────────────────────────────────────────────
    Pattern {
        category: "infrastructure",
        description: "Android ADB connection timed out",
        matches: &["ADBTimeoutError"],
        root_cause: "ADB lost connection to the Android test device. \
                     Can be caused by a device reboot, USB issue, or Bitbar hardware problem.",
        next_step: "Retrigger. If it fails consistently on the same device pool, \
                    report to the Bitbar/infra team.",
        fix_type: FixType::InfraReport,
        platform_hints: &["android"],
        example_bug: None,
    },

    // ── Browsertime: TypeError in samples (Android hardware) ─────────────────
    Pattern {
        category: "no_data",
        description: "Browsertime TypeError: Cannot read 'samples' — Android hardware issue",
        matches: &["TypeError: Cannot read properties of undefined", "samples"],
        root_cause: "Browsertime result data is missing the samples array. \
                     On Android this was linked to a faulty USB power meter on a specific \
                     device (Bug 1934169). The device was reporting incomplete data.",
        next_step: "Retrigger. If it's consistently failing on one Android platform, \
                    report to infra — the device hardware may need replacement.",
        fix_type: FixType::InfraReport,
        platform_hints: &["android"],
        example_bug: Some(1934169),
    },

    // ── Browsertime: composition recorder error ───────────────────────────────
    Pattern {
        category: "node_exception",
        description: "Browsertime: couldn't execute composition recorder script",
        matches: &["Couldn't execute async script named toggle composition recorder"],
        root_cause: "The video/profiler composition recorder script failed to execute. \
                     Likely a WebDriver timing issue or permissions problem.",
        next_step: "Retrigger. If persistent, check if the profiler extension is up to date \
                    and compatible with the current Firefox version.",
        fix_type: FixType::Retrigger,
        platform_hints: &["android"],
        example_bug: Some(1635749),
    },

    // ── ffmpeg failure ────────────────────────────────────────────────────────
    Pattern {
        category: "infrastructure",
        description: "ffmpeg video processing failed",
        matches: &["Command failed with exit code 1: ffmpeg"],
        root_cause: "ffmpeg failed to process the browsertime video recording. \
                     Likely a transient issue with the video file or ffmpeg availability.",
        next_step: "Retrigger. If ffmpeg is missing, run `perftest-brain doctor raptor` \
                    to check the local environment.",
        fix_type: FixType::Retrigger,
        platform_hints: &[],
        example_bug: Some(1641669),
    },

    // ── FailError: generic browsertime failure ────────────────────────────────
    Pattern {
        category: "browsertime_failed",
        description: "Browsertime FailError — internal browsertime failure",
        matches: &["\"name\":\"FailError\""],
        root_cause: "Browsertime's internal FailError was thrown. \
                     This is a catch-all for browsertime failures not covered by more specific errors.",
        next_step: "Check the full error message in the FailError JSON for specifics. \
                    Retrigger to confirm intermittency.",
        fix_type: FixType::Retrigger,
        platform_hints: &[],
        example_bug: Some(1647563),
    },

    // ── MutexImpl: pthread_mutex_lock failed ──────────────────────────────────
    Pattern {
        category: "infrastructure",
        description: "Firefox: pthread_mutex_lock failed — likely OOM or infra issue",
        matches: &["MutexImpl::mutexLock", "pthread_mutex_lock failed"],
        root_cause: "A Mutex lock operation failed inside Firefox, likely due to memory \
                     pressure or a system-level issue on the worker. \
                     High frequency (Bug 1777373, 328+ failures/week).",
        next_step: "Retrigger. This is a known infra-level issue (Bug 1777373). \
                    If frequency is high in your push, check for memory-intensive tests nearby.",
        fix_type: FixType::Retrigger,
        platform_hints: &[],
        example_bug: Some(1777373),
    },

    // ── ASAN/TSAN crashes ─────────────────────────────────────────────────────
    Pattern {
        category: "browser_crash",
        description: "AddressSanitizer or ThreadSanitizer crash detected",
        matches: &["SUMMARY: AddressSanitizer"],
        root_cause: "Firefox triggered an ASAN memory safety violation. \
                     This is a real Firefox bug (not a test intermittent) — \
                     the test found a memory error.",
        next_step: "File a browser crash bug with the full ASAN report. \
                    This is NOT a test intermittent — it requires a code fix. \
                    Use `perf-alert-cli` to find the culprit commit.",
        fix_type: FixType::FileCrashBug,
        platform_hints: &["linux"],
        example_bug: Some(2010150),
    },

    // ── Android mozperftest: task aborting ────────────────────────────────────
    Pattern {
        category: "infrastructure",
        description: "Android perftest task aborted (mozperftest tier-2)",
        matches: &["[taskcluster:error] Aborting task"],
        root_cause: "Android perftest task was aborted by Taskcluster. \
                     Often caused by hg clone getting stuck on the Android device (Bug 2038441), \
                     or the device running out of time before the test even starts.",
        next_step: "Retrigger. If it consistently fails on Android hardware workers, \
                    file a bug against Testing/mozperftest with the task ID.",
        fix_type: FixType::InfraReport,
        platform_hints: &["android"],
        example_bug: Some(2038441),
    },

    // ── CaR (Chromium-as-Release) failure — delegate to car-mechanic-cli ────────
    Pattern {
        category: "car",
        description: "CaR (Chromium-as-Release) build or test failure",
        matches: &["custom-car"],
        root_cause: "This is a CaR (Chromium-as-Release) failure. \
                     perftest-brain does not have CaR-specific pattern knowledge — \
                     car-mechanic does.",
        next_step: "Use car-mechanic for this failure: \
                    `car-mechanic diagnose --url <treeherder-url>`. \
                    car-mechanic encodes ~40 known CaR failure patterns with fix steps. \
                    Install: cargo install --git https://github.com/92kns/car-mechanic-cli",
        fix_type: FixType::CodeFix,
        platform_hints: &[],
        example_bug: None,
    },

    // ── Network error ─────────────────────────────────────────────────────────
    Pattern {
        category: "network",
        description: "Network error during page load (ERR_CONNECTION or similar)",
        matches: &["ERR_CONNECTION"],
        root_cause: "The test page URL returned a network error. \
                     Likely mitmproxy not serving the page correctly.",
        next_step: "Retrigger. Check if mitmproxy recording is up to date for this test page.",
        fix_type: FixType::Retrigger,
        platform_hints: &[],
        example_bug: None,
    },

    // ── mozperftest: NodeException (perma failure pattern) ────────────────────
    // From Bugs 1678588, 1703489: Perma NodeException failures fixed by code changes
    Pattern {
        category: "node_exception",
        description: "mozperftest NodeException: 1 — Node.js runner exited with error code 1",
        matches: &["mozperftest.test.browsertime.runner.NodeException: 1"],
        root_cause: "The mozperftest Node.js runner exited with code 1 (generic failure). \
                     If perma (not intermittent): likely a test change broke the script. \
                     If intermittent: device connectivity or transient Node crash.",
        next_step: "Check if this started after a test or harness change (Bug 1703489: \
                    fixed by Bug 1703070 harness fix). Retrigger once to confirm. \
                    If perma, look at the test script for a breaking change.",
        fix_type: FixType::CodeFix,
        platform_hints: &[],
        example_bug: Some(1703489),
    },

    // ── mozperftest: ModuleNotFoundError ──────────────────────────────────────
    // From Bugs 1675038, 1710578, 1717786: Python module missing after dependency change
    Pattern {
        category: "infrastructure",
        description: "Python ModuleNotFoundError in mozperftest — missing dependency",
        matches: &["ModuleNotFoundError: No module named"],
        root_cause: "A required Python module is missing from the test environment. \
                     Usually caused by a requirements.txt change not being synced, \
                     or a virtualenv not being rebuilt after a dependency update.",
        next_step: "This is typically a code fix: update requirements.txt or the vendored \
                    dependency. Check if a recent patch changed perftest dependencies without \
                    updating the test environment setup.",
        fix_type: FixType::CodeFix,
        platform_hints: &[],
        example_bug: Some(1710578),
    },

    // ── mozperftest: SSL certificate error ────────────────────────────────────
    // From Bug 1692467: controlled perftest ssl.SSLCertVerificationError
    Pattern {
        category: "infrastructure",
        description: "SSL certificate verification failed in controlled perftest",
        matches: &["SSLCertVerificationError", "CERTIFICATE_VERIFY_FAILED"],
        root_cause: "HTTPS connection failed due to certificate verification error. \
                     In controlled perftests this usually means the test server cert \
                     is self-signed and not trusted, or the cert expired.",
        next_step: "Check the controlled test server setup. \
                    If this is a known issue (Bug 1692467 pattern), consider disabling \
                    the affected test until the cert is fixed.",
        fix_type: FixType::CodeFix,
        platform_hints: &[],
        example_bug: Some(1692467),
    },

    // ── Perma failure: disable-recommended ───────────────────────────────────
    // From stockwell annotations on many bugs: [stockwell disable-recommended]
    Pattern {
        category: "perma_failure",
        description: "Test is perma-failing and marked disable-recommended by Stockwell",
        matches: &["stockwell disable-recommended"],
        root_cause: "Stockwell (the automated intermittent tracking system) has flagged this \
                     test as failing so frequently that it should be disabled. \
                     The test is hurting CI signal quality.",
        next_step: "Disable the test by adding `disabled = intermittent` to the raptor manifest, \
                    or add `skip-if = true` with a bug comment. \
                    File a bug (or use the existing one) tracking the underlying failure.",
        fix_type: FixType::SkipIf,
        platform_hints: &[],
        example_bug: None,
    },

    // ── Raptor: JavaScript error in addon during test ─────────────────────────
    // From Bug 1501040: Perma beta raptor-firefox-tp6 JavaScript error in XPIProvider
    Pattern {
        category: "node_exception",
        description: "JavaScript error in Firefox extension/addon during raptor test",
        matches: &["raptor", "JavaScript error:", "resource://"],
        root_cause: "A Firefox internal JavaScript error occurred during the test, \
                     likely in an extension or addon (XPIProvider, etc.). \
                     Can indicate a Firefox regression or an incompatible addon version.",
        next_step: "Check if this started after a Firefox change to the addon system. \
                    Retrigger to confirm intermittency. If perma, \
                    use `perf-alert-cli` to find the culprit commit.",
        fix_type: FixType::CodeFix,
        platform_hints: &[],
        example_bug: Some(1501040),
    },

    // ── mozperftest: JSONDecodeError in result processing ─────────────────────
    // From Bug 1767567: json.decoder.JSONDecodeError: Expecting value: line 1
    Pattern {
        category: "no_data",
        description: "mozperftest result processing failed: JSONDecodeError",
        matches: &["JSONDecodeError: Expecting value"],
        root_cause: "The test produced invalid or empty JSON output. \
                     Usually caused by the test process exiting before writing results, \
                     or a result file being empty/truncated.",
        next_step: "Check what the test process wrote to the result file. \
                    Retrigger once. If consistent, the test script may need a fix \
                    to handle error cases before writing results.",
        fix_type: FixType::CodeFix,
        platform_hints: &[],
        example_bug: Some(1767567),
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_patterns_have_non_empty_matches() {
        for p in PATTERNS {
            assert!(
                !p.matches.is_empty(),
                "Pattern {:?} has no match strings",
                p.description
            );
            for m in p.matches {
                assert!(
                    !m.is_empty(),
                    "Pattern {:?} has empty match string",
                    p.description
                );
            }
        }
    }

    #[test]
    fn real_log_lines_match_expected_patterns() {
        let test_cases = vec![
            // From Bug 1635752 log
            ("BrowserError: Could not start the browser with 3 tries", "browser_start"),
            // From Bug 1638702 / 1643581
            ("Exception: Browsertime failed to run", "browsertime_failed"),
            // From Bug 1641648 log
            ("Critical: Failed waiting on page to finished loading, timed out after 300000 ms", "timeout"),
            // From Bug 1499253 log
            ("TEST-UNEXPECTED-FAIL: no raptor test results were found for raptor-tp6", "no_data"),
            // From real AWSY job log (live data)
            ("marionette_driver.errors.InvalidSessionIdException awsy test", "browser_crash"),
            // From Bug 1809667
            ("Task aborted - max run time exceeded", "infrastructure"),
            // From real job log (live data 2026-06-09)
            ("error: no pipewire socket, retrying the task", "infrastructure"),
            // From Bug 1934169
            ("raptor-browsertime Critical: TypeError: Cannot read properties of undefined (reading 'samples')", "no_data"),
            // From Bug 1777373
            ("Hit MOZ_CRASH MutexImpl::mutexLock: pthread_mutex_lock failed", "infrastructure"),
            // From Bug 2010150
            ("SUMMARY: AddressSanitizer: SEGV in MOZ_CrashSequence", "browser_crash"),
        ];

        for (log_line, expected_category) in test_cases {
            let lower = log_line.to_lowercase();
            let matched = PATTERNS
                .iter()
                .find(|p| p.matches.iter().all(|m| lower.contains(&m.to_lowercase())));
            assert!(
                matched.is_some(),
                "No pattern matched log line: {:?}",
                log_line
            );
            assert_eq!(
                matched.unwrap().category,
                expected_category,
                "Wrong category for: {:?}",
                log_line
            );
        }
    }

    #[test]
    fn pattern_count_reflects_corpus_depth() {
        // We should have at least 20 grounded patterns
        assert!(
            PATTERNS.len() >= 20,
            "Expected ≥20 patterns, got {}",
            PATTERNS.len()
        );
    }
}
