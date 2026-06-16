<!-- GSD:project-start source:PROJECT.md -->

## Project

**perftest-brain**

A Rust CLI tool that acts as an embedded senior Firefox performance-test engineer. It diagnoses intermittently failing performance tests (starting with browsertime/raptor), generates fix patches written directly to a local Firefox checkout, and supports the sheriff workflow around triage and backout decisions. It lives alongside `perf-alert-cli` as a complementary tool — that tool investigates regressions; this one diagnoses harness failures and writes code.

**Core Value:** Given any intermittent failure signal (Treeherder job, test name/platform, Perfherder alert), produce a diagnosis and a ready-to-submit fix patch in the local Firefox checkout.

### Constraints

- **Language**: Rust — single binary, same pattern as car-mechanic-cli
- **Checkout detection**: Engineers run the tool from within their Firefox checkout — tool auto-detects checkout root from CWD. CLI flag/env var available as override; no hardcoded paths.
- **No home dir data**: Tool config/index lives in XDG config dir or alongside the project, not `~/.perftest-brain`
- **API dependencies**: Treeherder, Perfherder, Taskcluster APIs (same as perf-alert-cli); stmo-cli and searchfox-cli as subprocess calls
- **Patch scope**: Local file edits only — no Phabricator submission, no Try push (user handles that)

<!-- GSD:project-end -->

<!-- GSD:stack-start source:research/STACK.md -->

## Technology Stack

## Executive Summary

## Recommended Stack

### CLI Framework

| Crate | Version | Why |
|-------|---------|-----|
| `clap` (features = `["derive"]`) | `4.6` | Industry standard, derive macros give declarative subcommand structure, matches `car-mechanic-cli` exactly. Subcommand-per-domain (`diagnose`, `update`, `doctor`, `triage`, `patch`) is the same architecture PROJECT.md calls out as proven. |

### HTTP Client

| Crate | Version | Why |
|-------|---------|-----|
| `ureq` | `3.x` (latest `3`) | Blocking, synchronous, minimal-dependency HTTP client. Perfect fit for sequential API calls (Treeherder/Perfherder/Taskcluster). No async runtime dragged in. `car-mechanic-cli` uses `ureq` 2.x in production. **Adopt 3.x for new code** — 3.x has much stricter semver adherence (2.x's mistake was re-exporting pre-1.0 crates like `rustls`/`cookie`), and shims TLS/cookie config behind its own types so dependency churn won't break you. |
| `url` | `2.5` | Build/parse Treeherder & Taskcluster query URLs robustly instead of string concatenation. |

### Async Runtime

| Decision | Rationale |
|----------|-----------|
| No `tokio`, no `async-std` | The tool's I/O is naturally sequential: fetch a job → fetch its log → query an API → invoke a subprocess → write a file. There is no fan-out workload that benefits from async. `reqwest`'s blocking mode *still spawns a tokio runtime thread*, which is pure overhead here. Synchronous code is simpler to read, test, and debug — and keeps the binary small. |

### Serialization

| Crate | Version | Why |
|-------|---------|-----|
| `serde` (features = `["derive"]`) | `1.0` | Universal. Derive `Deserialize` for API response types (Treeherder jobs, Perfherder alerts, Taskcluster tasks). |
| `serde_json` | `1.0` | All three APIs are JSON. Also the format for `--json` machine-readable output (car-mechanic has a global `--json` flag — replicate it for composability with `perf-alert-cli`/agents). |
| `toml` | `0.9` | Config file format (Firefox checkout path, API tokens/endpoints, index location). TOML is the Rust-ecosystem standard for human-edited config. |
| `chrono` | `0.4` | Parse/format timestamps from Treeherder/Perfherder (alert push times, job durations). Use `features = ["serde"]` for direct deserialization. |

### File System / Indexing

| Crate | Version | Why |
|-------|---------|-----|
| `ignore` | `0.4` | **Walk the Firefox checkout, not `walkdir`.** The checkout is enormous (~400k files) and has `.gitignore`/`.hgignore`. `ignore` (from ripgrep) respects ignore files, skips `.git`/`obj-*` build dirs automatically, and provides a parallel walker. Far better than `walkdir` for scanning a source tree of this size. |
| `rusqlite` (features = `["bundled"]`) | `0.37` | **Persistent local index store.** PROJECT.md requires `perftest-brain update` to build a queryable index of test metadata (raptor/mozperftest test structure). SQLite is the right call: fast indexed lookups, single-file, queryable, survives between runs. `bundled` feature statically links SQLite → keeps the single-binary promise (no system libsqlite dependency). |
| `directories` | `6.x`/`5.x` | **XDG-correct paths.** PROJECT.md explicitly forbids `~/.perftest-brain` and hardcoded `~/` paths. `directories::ProjectDirs` gives the correct `$XDG_CONFIG_HOME` / `$XDG_DATA_HOME` (and macOS equivalents) for config and the SQLite index. This directly satisfies a hard constraint. |
| `tempfile` | `3.x` | **Atomic patch writes.** When generating fix patches into the live Firefox checkout, write to a temp file in the same directory and atomically rename over the target. Prevents leaving a half-written source file if the process dies mid-write — critical when editing someone's working tree. |
| `walkdir` | `2.5` | *Only if* you need a simple recursive walk of a small, self-owned directory (e.g., the tool's own data dir). For the Firefox checkout, prefer `ignore`. |

### Subprocess Management

| Approach | Why |
|----------|-----|
| `std::process::Command` (std lib) | **No crate needed.** This is exactly how `car-mechanic-cli` invokes `treeherder-cli`, `chromium-search`, and `python3`. Use it for `searchfox-cli`, `stmo-cli`, `perf-alert-cli`, `profiler-cli`. Capture stdout/stderr, check exit status, surface failures via `anyhow` context. |

- Capture `.output()` for tools you parse (searchfox/stmo JSON), use `.status()` for fire-and-forget.
- Wrap missing-binary errors with `anyhow::Context` → a clear "is `searchfox-cli` on PATH?" message. The tool depends on 4 external binaries; failure messages must name the missing tool.
- PROJECT.md mandates **searchfox-cli as a fallback** when the local index is insufficient — model this as: query `rusqlite` index first, fall back to a `searchfox-cli` subprocess on miss.

### Code Generation / Patching

| Crate | Version | Why |
|-------|---------|-----|
| `similar` | `2.7` | **Diff display.** Generate and show unified diffs of proposed patches before/after writing (`similar` powers `insta` and many diff viewers; supports unified-diff output and inline/word-level highlighting). Use it to render "here's what I'll change" to the user, and for `--json` diff payloads. |
| `regex` | `1.x` | Targeted edits to known config/manifest structures (raptor `.ini`/manifest entries, `perftest.toml`, browsertime args). Matches car-mechanic usage. |
| **`tree-sitter` — DEFER (see below)** | `0.25`/`0.26` | Only adopt if you need *structural* edits to Python/JS test harness code. |

### Error Handling

| Crate | Version | Why |
|-------|---------|-----|
| `anyhow` | `1.0` | **Application-level errors.** This is a binary, not a library — `anyhow::Result<T>` + `.context(...)` is the idiomatic, ergonomic choice. Matches `car-mechanic-cli`. Rich context chains are exactly what you want for "diagnose why the API call / subprocess / file write failed." |
| `thiserror` | `2.x` | **Only if** a phase introduces a reusable internal library crate with typed errors callers must match on (e.g., distinguishing `NotFound` vs `RateLimited` vs `Network` for retry logic). For a single-binary tool, `anyhow` alone is usually enough; add `thiserror` surgically. |

### Logging / Diagnostics

| Crate | Version | Why |
|-------|---------|-----|
| `tracing` + `tracing-subscriber` | `0.1` / `0.3` | Structured, leveled diagnostics behind a `-v/--verbose` flag. Valuable here because the tool orchestrates many API/subprocess steps — being able to trace "which call failed and what it returned" is essential for a diagnosis tool. `tracing` over `log`+`env_logger` because it gives spans (group all work for one `diagnose` invocation) and structured fields. |

### Terminal UX

| Crate | Version | Why | When |
|-------|---------|-----|------|
| `indicatif` | `0.17` | Progress bar/spinner for `perftest-brain update` (indexing 400k files takes seconds–minutes; users need feedback). | Adopt for the `update` command. |
| `dialoguer` | `0.11` | Interactive confirmation before writing patches to the live checkout ("Apply this patch to `testing/raptor/...`? [y/N]") and for selecting among candidate diagnoses. | Adopt for patch-apply confirmation. Pair with a `--yes`/`--no-input` flag for non-interactive/agent use. |
| `comfy-table` | `7.x` | Render triage/grooming tables (alert backlog, tier classification, owner suggestions) for human-readable output. | Adopt for `triage`/grooming output; gate behind non-`--json` mode. |
| `owo-colors` or `anstream`/`anstyle` | latest | Colored severity output (tier 1/2/3, pass/fail). `clap` 4 already pulls `anstyle`; reuse it. | Optional polish. |

### Testing

| Crate | Version | Why |
|-------|---------|-----|
| `assert_cmd` | `2.x` | Integration-test the binary's CLI surface (run subcommands, assert exit codes/stdout). Standard for clap-based CLIs. |
| `predicates` | `3.x` | Assertion helpers for `assert_cmd` output matching. |
| `insta` | `1.x` | **Snapshot testing** for diagnosis output and generated patches. Ideal here: diagnosis reports and patch diffs are large structured text — snapshot tests catch regressions without hand-writing brittle assertions. Built on `similar` (same diff engine you're already using). |
| `mockito` | `1.x` | Mock Treeherder/Perfherder/Taskcluster HTTP responses in tests so the suite is hermetic (no live API dependency). Works with `ureq` (point the client at a local mock URL). |
| `tempfile` | `3.x` | Spin up throwaway dirs to test indexing and patch-writing against a fake checkout. (Same crate as the runtime atomic-write use.) |

## Suggested `Cargo.toml` (starting point)

# Add ONLY if a phase justifies it:

# thiserror = "2"          # typed errors in an extracted lib crate

# tree-sitter = "0.25"     # AST edits to Python/JS harness source

# tree-sitter-python = "0.23"

# rayon = "1"              # parallel indexing if `update` is too slow serially

## What NOT to Use

| Avoid | Why |
|-------|-----|
| **`tokio` / async runtime** | No concurrency pressure. The workload is sequential API + subprocess calls. Async adds compile time, binary bloat, and reasoning overhead for zero benefit. car-mechanic-cli proved synchronous works. |
| **`reqwest`** | Pulls in `tokio` (even `reqwest::blocking` spins up a runtime thread). Use `ureq` for blocking HTTP. The retry-middleware ecosystem (`reqwest-retry`/`reqwest-middleware`) isn't worth dragging the whole async stack in — hand-roll backoff over `ureq`. |
| **`argh` / `structopt` / `getopts`** | `clap` derive is the standard and matches the sibling tool. `structopt` is deprecated (merged into clap 4). `argh` is minimalist but lacks clap's subcommand ergonomics this tool needs. |
| **`walkdir` for the Firefox checkout** | Doesn't respect `.gitignore`/`.hgignore`; will waste time descending into `.git`, `obj-*` build artifacts, `node_modules`. Use `ignore` for the checkout. `walkdir` is fine only for small self-owned dirs. |
| **Flat-JSON index file** | Re-reading/parsing a large JSON blob on every lookup is slow and unindexed. `rusqlite` gives proper indexed queries and incremental updates. (See Open Questions — JSON acceptable only if the index stays tiny.) |
| **`tree-sitter` in v1 (speculative)** | Don't pay for an AST parser until a patch genuinely needs structural Python/JS edits. v1 patches are config/manifest/INI/TOML-shaped → `regex` + `toml` suffice. |
| **`failure`, `error-chain`** | Obsolete error crates. Use `anyhow` (+ `thiserror` if needed). |
| **`std::fs::write` directly onto checkout files** | Non-atomic; a crash mid-write corrupts the user's source. Always write-temp-then-rename via `tempfile` in the target directory. |
| **Hardcoded `~/.perftest-brain` paths** | Explicitly forbidden by PROJECT.md constraints. Use `directories` for XDG/platform-correct config + data dirs. |
| **Bundling external tools as crates** | `searchfox-cli`/`stmo-cli`/`perf-alert-cli`/`profiler-cli` are separate binaries — call them via `std::process::Command`, don't try to vendor or FFI them. |

## Open Questions

## Confidence Assessment

| Area | Confidence | Reason |
|------|------------|--------|
| CLI framework (`clap`) | HIGH | Direct car-mechanic precedent; no alternative. |
| HTTP (`ureq` 3.x, no async) | HIGH | Sibling-tool precedent + 2025 ecosystem consensus for blocking CLI HTTP. |
| Async decision (none) | HIGH | Workload is sequential; async unjustified. |
| Serialization (`serde`/`json`/`toml`) | HIGH | Universal standard. |
| Indexing (`ignore`+`rusqlite`+`directories`) | HIGH (crates) / MEDIUM (SQLite-vs-JSON) | Crate choices solid; storage shape needs a spike. |
| Subprocess (`std::process`) | HIGH | Proven in car-mechanic for the same external-tool pattern. |
| Patching (`similar`+`regex`, defer `tree-sitter`) | HIGH (v1) / MEDIUM (tree-sitter deferral) | Depends on patch complexity, unknown until that phase. |
| Error handling (`anyhow`) | HIGH | Idiomatic for binaries; car-mechanic precedent. |
| Terminal UX | HIGH (choices) / MEDIUM (necessity) | Right crates; per-phase priority varies. |
| Testing | HIGH | Consensus 2025 Rust CLI test stack. |

- `car-mechanic-cli` Cargo.toml + src (`/Users/kshampur/symposium/car-mechanic-cli`) — production sibling tool, MPL-2.0 (HIGH confidence, direct precedent)
- Local cargo registry cache (`~/.cargo/registry`) — current resolved crate versions as of 2026-06 (HIGH)
- [ureq GitHub / lib.rs](https://lib.rs/crates/ureq) and [How to choose the right Rust HTTP client — LogRocket](https://blog.logrocket.com/best-rust-http-client/) — ureq 3.x semver/positioning, ureq-vs-reqwest blocking guidance (HIGH)
- [tree-sitter on docs.rs](https://docs.rs/crate/tree-sitter/latest) and [crates.io](https://crates.io/crates/tree-sitter) — current tree-sitter versions (0.25.x stable, 0.26.x) (HIGH)

<!-- GSD:stack-end -->

<!-- GSD:conventions-start source:CONVENTIONS.md -->

## Conventions

Conventions not yet established. Will populate as patterns emerge during development.
<!-- GSD:conventions-end -->

<!-- GSD:architecture-start source:ARCHITECTURE.md -->

## Architecture

Architecture not yet mapped. Follow existing patterns found in the codebase.
<!-- GSD:architecture-end -->

<!-- GSD:skills-start source:skills/ -->

## Project Skills

No project skills found. Add skills to any of: `.claude/skills/`, `.agents/skills/`, `.cursor/skills/`, `.github/skills/`, or `.codex/skills/` with a `SKILL.md` index file.
<!-- GSD:skills-end -->

<!-- GSD:workflow-start source:GSD defaults -->

## GSD Workflow Enforcement

Before using Edit, Write, or other file-changing tools, start work through a GSD command so planning artifacts and execution context stay in sync.

Use these entry points:

- `/gsd-quick` for small fixes, doc updates, and ad-hoc tasks
- `/gsd-debug` for investigation and bug fixing
- `/gsd-execute-phase` for planned phase work

Do not make direct repo edits outside a GSD workflow unless the user explicitly asks to bypass it.
<!-- GSD:workflow-end -->

<!-- GSD:profile-start -->

## Developer Profile

> Profile not yet configured. Run `/gsd-profile-user` to generate your developer profile.
> This section is managed by `generate-claude-profile` -- do not edit manually.
<!-- GSD:profile-end -->
