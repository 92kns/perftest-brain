/// A failure signature pattern — matches log text to a known failure category.
#[derive(Debug, Clone)]
pub struct Pattern {
    pub category: &'static str,
    pub description: &'static str,
    /// Substrings that must all appear in the log to match this pattern.
    pub matches: &'static [&'static str],
    /// The root cause explanation surfaced to the user.
    pub root_cause: &'static str,
    /// Recommended next step.
    pub next_step: &'static str,
}

/// Known browsertime/raptor/mozperftest intermittent failure patterns.
/// Seeded from the Bugzilla corpus of recurring failure signatures.
pub static PATTERNS: &[Pattern] = &[
    Pattern {
        category: "timeout",
        description: "Browsertime timed out waiting for page load or script",
        matches: &["browsertime timed out", "Navigation timeout"],
        root_cause: "The test exceeded its timeout budget. Common causes: slow network, heavy page, or startup regression.",
        next_step: "Add `requestLongerTimeout(2)` in the test manifest, or investigate if a recent patch slowed startup.",
    },
    Pattern {
        category: "timeout",
        description: "Raptor test timeout",
        matches: &["raptor timed out", "Test timed out"],
        root_cause: "Raptor test exceeded its allowed duration.",
        next_step: "Check if the test needs a longer timeout in raptor/raptor.ini, or investigate a performance regression.",
    },
    Pattern {
        category: "no_data",
        description: "No browsertime data collected",
        matches: &["No data to collect", "browsertime result file"],
        root_cause: "Browsertime ran but produced no measurement output. Browser may have crashed or the test URL was unreachable.",
        next_step: "Check the full job log for a browser crash or network error. Consider adding a `skip-if` for the failing platform.",
    },
    Pattern {
        category: "no_data",
        description: "Empty perfherder data",
        matches: &["PERFHERDER_DATA", "no results"],
        root_cause: "Test ran but emitted no Perfherder data blob. Test harness may have failed before measurements were taken.",
        next_step: "Look for earlier errors in the log that caused premature exit.",
    },
    Pattern {
        category: "node_exception",
        description: "Browsertime Node.js exception",
        matches: &["NodeException", "UnhandledPromiseRejection"],
        root_cause: "Browsertime's Node.js runner threw an unhandled exception during test execution.",
        next_step: "Check if the test script references an API that changed, or if a dependency update broke compatibility.",
    },
    Pattern {
        category: "browser_crash",
        description: "Browser crashed during test",
        matches: &["CRASH", "minidump", "ExceptionCode"],
        root_cause: "Firefox crashed during the performance test run.",
        next_step: "File a browser crash bug with the minidump. Check if a recent patch introduced instability.",
    },
    Pattern {
        category: "infrastructure",
        description: "Worker infrastructure failure",
        matches: &["Worker exception", "task exception", "worker-shutdown"],
        root_cause: "The CI worker itself had a problem unrelated to the test code.",
        next_step: "Retrigger the job — this is likely a transient infrastructure issue, not a code problem.",
    },
    Pattern {
        category: "startup",
        description: "Firefox failed to start",
        matches: &["Firefox failed to start", "application crashed at startup", "GeckoDriver"],
        root_cause: "Firefox could not be launched by the test harness.",
        next_step: "Check for a GeckoDriver version mismatch or a startup crash. Retrigger to confirm intermittency.",
    },
    Pattern {
        category: "network",
        description: "Network error during test",
        matches: &["net::ERR_", "ERR_CONNECTION", "ECONNREFUSED"],
        root_cause: "Test required a network resource that was unreachable.",
        next_step: "Check if the test depends on an external URL. Consider adding a `skip-if` or using a local fixture.",
    },
    Pattern {
        category: "missing_profile",
        description: "Missing Gecko profiler profile",
        matches: &["profile not found", "profiler", "profile.zip"],
        root_cause: "The Gecko profiler did not produce output, likely because the test failed before profiling completed.",
        next_step: "Resolve the underlying test failure first; the missing profile is a symptom.",
    },
];
