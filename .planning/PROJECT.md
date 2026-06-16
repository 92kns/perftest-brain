# perftest-brain

## What This Is

A Rust CLI tool that acts as an embedded senior Firefox performance-test engineer. It diagnoses intermittently failing performance tests (starting with browsertime/raptor), generates fix patches written directly to a local Firefox checkout, and supports the sheriff workflow around triage and backout decisions. It lives alongside `perf-alert-cli` as a complementary tool — that tool investigates regressions; this one diagnoses harness failures and writes code.

## Core Value

Given any intermittent failure signal (Treeherder job, test name/platform, Perfherder alert), produce a diagnosis and a ready-to-submit fix patch in the local Firefox checkout.

## Requirements

### Validated

(None yet — ship to validate)

### Active

- [ ] Diagnose browsertime intermittent failures from a Treeherder job URL, test name + platform pattern, or Perfherder alert
- [ ] Generate fix patches for intermittents written directly to the user's local Firefox checkout
- [ ] `perftest-brain update` command that indexes test structure from the Firefox checkout
- [ ] Searchfox-cli fallback when local index is missing or insufficient
- [ ] Sheriff support: tier 1/2/3 classification, backout recommendation, regression summary
- [ ] Grooming / triage assistant: summarize open alert backlog, prioritize, suggest owners
- [ ] Tool doctor for raptor and mozperftest: diagnose harness-level setup and runtime failures
- [ ] Leverage stmo-cli for historical query data on test signal quality

### Out of Scope

- Absorbing perf-alert-cli regression investigation — that tool remains separate and complementary
- GUI or web interface — CLI only
- Automatic try run submission — local patches only; user pushes to Phabricator/Try manually
- Tool doctor for awsy/talos in v1 — raptor + mozperftest first
- Hardcoded `~/` paths — Firefox checkout path provided via flag or config; tool is portable
- Writing to `~` home directory for config/data — use XDG config or a path within the project

## Context

- **Prior art**: `perf-alert-cli` (TypeScript, npm) handles alert investigation and culprit commit ranking. `perftest-brain` handles the other half: intermittent diagnosis + patch generation. Both can be used together in a full sheriffing session.
- **Previous exploration**: `/Users/kshampur/symposium/perf-alert-cli` has working TypeScript patterns for Perfherder/Treeherder/Taskcluster API calls that can be referenced for port to Rust.
- **Skill WIP**: `/Users/kshampur/firefox/.claude/skills/perf-alert/SKILL.md` documents the investigation workflow Claude currently follows manually — this brain automates it.
- **Firefox source**: `/Users/kshampur/firefox` — relevant paths: `testing/raptor/`, `testing/talos/`, `testing/awsy/`, `testing/mozperftest/`, `testing/performance/`, `taskcluster/` for CI task definitions.
- **Treeherder**: local clone at `~/perf/treeherder`, GitHub at https://github.com/mozilla/treeherder
- **Knowledge freshness**: Local index built by `perftest-brain update` (walks Firefox checkout); searchfox-cli queried at runtime as fallback for things not indexed.
- **External tools available**: `searchfox-cli`, `stmo-cli` (STMO API key configured), `perf-alert-cli`, `profiler-cli`
- **car-mechanic-cli** pattern: Rust single binary with specialized subcommands per domain — same architecture applies here. Worked well.

## Constraints

- **Language**: Rust — single binary, same pattern as car-mechanic-cli
- **Portability**: Firefox checkout path must be user-configurable; no hardcoded `~/` paths in the tool
- **No home dir data**: Tool config/index lives in XDG config dir or alongside the project, not `~/.perftest-brain`
- **API dependencies**: Treeherder, Perfherder, Taskcluster APIs (same as perf-alert-cli); stmo-cli and searchfox-cli as subprocess calls
- **Patch scope**: Local file edits only — no Phabricator submission, no Try push (user handles that)

## Key Decisions

| Decision | Rationale | Outcome |
|----------|-----------|---------|
| Rust (not TypeScript) | Single binary, car-mechanic-cli precedent, better CLI ergonomics | — Pending |
| Parallel tool to perf-alert-cli (not absorbing it) | Different focus domains; Rust/TypeScript boundary makes subprocess calls awkward | — Pending |
| Local index + searchfox-cli fallback | Speed for common lookups, always-current for edge cases | — Pending |
| raptor + mozperftest first (not all 4 harnesses) | These are the primary pain points; awsy/talos deferred | — Pending |
| Patch written to local checkout (not diff output) | User workflow ends at `arc submit` / Try push; in-place edits are more natural | — Pending |

---
*Last updated: 2026-06-16 after initialization*

## Evolution

This document evolves at phase transitions and milestone boundaries.

**After each phase transition** (via `/gsd-transition`):
1. Requirements invalidated? → Move to Out of Scope with reason
2. Requirements validated? → Move to Validated with phase reference
3. New requirements emerged? → Add to Active
4. Decisions to log? → Add to Key Decisions
5. "What This Is" still accurate? → Update if drifted

**After each milestone** (via `/gsd-complete-milestone`):
1. Full review of all sections
2. Core Value check — still the right priority?
3. Audit Out of Scope — reasons still valid?
4. Update Context with current state
