# perftest-brain — AI Agent Guide

perftest-brain is a CLI tool that acts as an embedded senior Firefox performance-test engineer. It diagnoses intermittent test failures, generates fix patches, and supports the sheriff workflow.

Run from inside a mozilla-central checkout. All commands accept `--json` for machine-readable output.

> For AI agents: pipe this file into your context with `perftest-brain agents`.

## Input Formats

Every command that takes `<input>` accepts any of these:

| Format | Example |
|--------|---------|
| Perfherder alert ID | `44793` |
| Perfherder alert URL | `https://treeherder.mozilla.org/perfherder/alerts?id=44793` |
| Bugzilla bug URL | `https://bugzilla.mozilla.org/show_bug.cgi?id=2042450` |
| Treeherder push URL | `https://treeherder.mozilla.org/jobs?repo=autoland&revision=abc123` |
| PerfCompare URL | `https://perf.compare.firefox.com/?baseRev=abc&newRev=def&...` |
| Revision hash | `4bfc5585ab5d` (12–40 hex chars) |

## Commands

### `diagnose <input>` — diagnose an intermittent failure

```
perftest-brain diagnose 44793
perftest-brain diagnose 'https://treeherder.mozilla.org/perfherder/alerts?id=44793'
perftest-brain diagnose --json 44793
```

Output: signal type (intermittent vs sustained regression), failure category, root cause, Bugzilla lookup, stmo-cli noise context, next steps.

Signal types:
- `intermittent` — frequency-based, high-variance, appears and disappears randomly
- `sustained_regression` — Perfherder t-test verdict, requires culprit commit investigation via `perf-alert-cli`
- `inconclusive` — not enough data; retrigger the job and try again

Exit codes: `0` success, `1` error.

### `patch <input>` — generate and apply a fix patch

```
perftest-brain patch 44793
perftest-brain patch --json 44793   # non-interactive (auto-applies)
```

Flow: diagnose → VCS dirty-state check → determine fix → write patch to local checkout.

Refuses to patch if the working copy is dirty. Patches target:
- `requestLongerTimeout(N)` for timeout intermittents
- `skip-if` conditions for platform-specific failures

### `sheriff <input>` — tier classification and backout recommendation

```
perftest-brain sheriff 44793
perftest-brain sheriff --json 44793
```

Output: Tier 1/2/3 classification with reasoning, backout recommendation with explicit justification, affected platforms.

Tiers:
- **Tier 1** — ≥10% regression on critical framework + primary test or multi-platform. Backout recommended.
- **Tier 2** — ≥5% regression on critical framework, or multi-platform. Investigate within 24h.
- **Tier 3** — Small regression or noise. Monitor.

### `groom` — rank open alert backlog

```
perftest-brain groom
perftest-brain groom --json
```

Fetches untriaged browsertime alerts from Perfherder, ranks by tier × severity, suggests owners.

### `doctor <harness>` — local harness health check

```
perftest-brain doctor raptor
perftest-brain doctor mozperftest
perftest-brain doctor raptor --json
```

Checks required tools and files for the given harness. Supported: `raptor`, `mozperftest`.

### `update` — update perftest-brain to the latest version

```
perftest-brain update
```

Self-update via `cargo install --force --git https://github.com/92kns/perftest-brain`. Checks the latest GitHub tag first and skips if already up to date.

### `reindex` — rebuild the local test index

```
perftest-brain reindex
perftest-brain reindex --json
```

Walks `testing/raptor/`, `testing/mozperftest/`, `testing/performance/`, and `taskcluster/` in the checkout and writes an incremental SQLite index to the XDG data directory. Falls back to `searchfox-cli` when the index is insufficient.

### `info <input>` — show resolved signal info

```
perftest-brain info 44793
perftest-brain info --json 44793
```

Resolves any supported input to a structured summary without full diagnosis.

### `agents` — print this guide to stdout

```
perftest-brain agents
perftest-brain agents | <your-ai-tool>
```

## Global Flags

| Flag | Description |
|------|-------------|
| `--json` | Machine-readable JSON on all commands; errors as `{"error":"...","exit_code":1}` |
| `-v` / `--verbose` | Debug output; `-vv` includes subprocess stdout/stderr |
| `--checkout-path <PATH>` | Override Firefox checkout auto-detection |

## Checkout Detection

Auto-detects the Firefox checkout root by walking up from CWD, requiring both a `mach` file AND `.hg`/`.git` (both required — any git repo does not qualify). Override with `--checkout-path` or `PERFTEST_BRAIN_CHECKOUT` env var.

## Failure Pattern Knowledge Base

The `diagnose` command matches against a corpus of real failure patterns grounded in:
- Treeherder failures API (autoland, 6 weeks of data, top recurring bugs)
- Bugzilla Testing/Raptor and Testing/mozperftest intermittent-failure bug corpus
- Live Taskcluster job logs from failed tasks

Each pattern includes:
- `category` — failure type (browser_crash, timeout, no_data, infrastructure, etc.)
- `fix_type` — what kind of fix is typically needed (Retrigger, RequestLongerTimeout, SkipIf, FileCrashBug, InfraReport, CodeFix)
- `platform_hints` — platforms where this pattern is most common
- `example_bug` — a representative Bugzilla bug ID

### High-frequency patterns (real data, last 6 weeks on autoland)

| Bug | Failures/week | Pattern | Fix type |
|-----|---------------|---------|----------|
| [1809667](https://bugzilla.mozilla.org/show_bug.cgi?id=1809667) | 815 | `Task aborted - max run time exceeded` | Retrigger / InfraReport |
| [1358898](https://bugzilla.mozilla.org/show_bug.cgi?id=1358898) | 836 | `RunWatchdog` — Firefox kills itself on shutdown | Retrigger |
| [2038441](https://bugzilla.mozilla.org/show_bug.cgi?id=2038441) | 291 | Android perftest `[taskcluster:error] Aborting task` (hg clone stuck) | InfraReport |
| [1777373](https://bugzilla.mozilla.org/show_bug.cgi?id=1777373) | 328 | `MutexImpl::mutexLock: pthread_mutex_lock failed` | Retrigger |
| [1934169](https://bugzilla.mozilla.org/show_bug.cgi?id=1934169) | ~231 | `TypeError: Cannot read properties of undefined (reading 'samples')` on Android | InfraReport |
| [1635752](https://bugzilla.mozilla.org/show_bug.cgi?id=1635752) | – | `BrowserError: Could not start the browser with 3 tries` | Retrigger / doctor |

### Diagnosis output fields

`diagnose --json` returns findings with:
```json
{
  "category": "timeout",
  "fix_type": "RequestLongerTimeout",
  "platform_hints": [],
  "example_bug": 1641648,
  "next_step": "Add requestLongerTimeout to the test manifest..."
}
```

Use `fix_type` to decide what `patch` will do:
- `RequestLongerTimeout` → `patch` adds `requestLongerTimeout` to the manifest
- `SkipIf` → `patch` adds a `skip-if` condition
- `Retrigger` / `InfraReport` → no patch; retrigger or file infra bug
- `FileCrashBug` → `patch` won't help; open a browser crash bug

## External Tools

Calls these when available; gracefully degrades when absent:

| Tool | Used by | Notes |
|------|---------|-------|
| `stmo-cli` | `diagnose` | Historical noise context for signal quality |
| `searchfox-cli` | `diagnose`, `update` | Code search fallback when local index is empty |
| `perf-alert-cli` | — | Companion tool for regression culprit investigation |
| `git` / `hg` / `jj` | `patch` | VCS dirty-state check before patching |

## Typical Workflows

**Diagnose and patch a failing job:**
```
perftest-brain diagnose 44793
perftest-brain patch 44793
hg diff   # or git diff
```

**Sheriff triage:**
```
perftest-brain sheriff 44793
perftest-brain groom
```

**AI agent usage:**
```
perftest-brain diagnose --json 44793 | <ai-tool>
perftest-brain agents | <ai-tool>
```
