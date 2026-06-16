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

### `update` — rebuild the local test index

```
perftest-brain update
perftest-brain update --json
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
