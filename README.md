# perftest-brain

Routes Firefox performance failure signals — Perfherder alerts, Treeherder job URLs, test names — through diagnosis, patch generation, and sheriff triage. Encodes harness-specific knowledge for raptor, mozperftest, and browsertime, and integrates with treeherder-cli, stmo-cli, and searchfox-cli already in your Firefox checkout.

Designed for use by both engineers and AI agents.

> For AI agents: see [AGENTS.md](AGENTS.md) for the full workflow guide, or run `perftest-brain agents`.

## Install

Requires [Rust](https://rustup.rs).

```
cargo install --git https://github.com/92kns/perftest-brain
```

Run from inside a mozilla-central checkout. The tool auto-detects the checkout root.

## Commands

### `diagnose` — identify what went wrong

```
perftest-brain diagnose 44793
perftest-brain diagnose 'https://treeherder.mozilla.org/perfherder/alerts?id=44793'
perftest-brain diagnose --json 44793
```

Takes any Perfherder alert ID, Treeherder URL, Bugzilla URL, or revision hash. Returns:
- Signal type: intermittent vs sustained regression
- Failure category and root cause (timeout, no-data, browser crash, infra, etc.)
- Existing Bugzilla bugs for the failing test
- Historical noise context via stmo-cli (if available)
- Recommended next steps

### `patch` — write a fix to the local checkout

```
perftest-brain patch 44793
```

Diagnoses the signal, checks the working copy is clean, then writes an appropriate fix:
- `requestLongerTimeout(2)` for timeout failures
- `skip-if` for platform-specific intermittents

Atomic write — refuses to patch a dirty working copy.

### `sheriff` — tier classification and backout recommendation

```
perftest-brain sheriff 44793
```

Returns Tier 1/2/3 classification with explicit reasoning and a backout recommendation.

| Tier | Threshold | Action |
|------|-----------|--------|
| 1 | ≥10% on critical framework, primary test or multi-platform | Backout recommended |
| 2 | ≥5% on critical framework | Investigate within 24h |
| 3 | Small or noise | Monitor |

### `groom` — rank the open alert backlog

```
perftest-brain groom
```

Fetches untriaged browsertime alerts from Perfherder, ranks by priority score (tier × severity), and suggests owners.

### `doctor` — harness health check

```
perftest-brain doctor raptor
perftest-brain doctor mozperftest
```

Checks that all required tools and files are present for the given harness.

### `update` — update perftest-brain

```
perftest-brain update
```

Self-update to the latest version via `cargo install --force --git`.

### `reindex` — rebuild the local test index

```
perftest-brain update
```

Walks `testing/raptor/`, `testing/mozperftest/`, `testing/performance/`, and `taskcluster/` and writes an incremental SQLite index. Used by `diagnose` for fast test lookups; falls back to `searchfox-cli` when the index is empty.

### `info` — inspect a signal

```
perftest-brain info 44793
perftest-brain info 'https://bugzilla.mozilla.org/show_bug.cgi?id=2042450'
```

Resolves any supported input to a structured summary without full diagnosis.

## Input Formats

All commands that take `<input>` accept:

| Format | Example |
|--------|---------|
| Alert ID | `44793` |
| Perfherder URL | `https://treeherder.mozilla.org/perfherder/alerts?id=44793` |
| Bugzilla URL | `https://bugzilla.mozilla.org/show_bug.cgi?id=2042450` |
| Treeherder push URL | `https://treeherder.mozilla.org/jobs?repo=autoland&revision=abc123` |
| PerfCompare URL | `https://perf.compare.firefox.com/?baseRev=abc&newRev=def` |
| Revision hash | `4bfc5585ab5d` |

## Global Flags

| Flag | Description |
|------|-------------|
| `--json` | Machine-readable JSON output; errors as `{"error":"...","exit_code":1}` |
| `-v` | Verbose output; `-vv` includes subprocess stdout/stderr |
| `--checkout-path <PATH>` | Override checkout auto-detection |

## Dependencies

| Tool | Required | Notes |
|------|----------|-------|
| `stmo-cli` | Optional | Historical noise context for `diagnose` |
| `searchfox-cli` | Optional | Code search fallback for `update`/`diagnose` |
| `perf-alert-cli` | Optional | Companion tool — regression culprit investigation |
| `git` / `hg` / `jj` | Optional | VCS dirty-state check for `patch` |

All external tools degrade gracefully when not found.

## For AI Agents

All commands support `--json`. Feed the agent guide into your AI context:

```
perftest-brain agents | <your-ai-tool>
```

Or see [AGENTS.md](AGENTS.md) directly.
