---
status: archived
created: 2026-03-22
priority: high
tags:
- ci
- feedback-loop
- github-actions
- factory
created_at: 2026-03-22T14:47:11.571307991Z
updated_at: 2026-03-22T16:43:10.155396306Z
transitions:
- status: in-progress
  at: 2026-03-22T14:53:04.816483986Z
- status: archived
  at: 2026-03-22T16:43:10.155396306Z
---

# Auto-Close Feedback Loop: CI Failure → Fix → Re-run

## Overview

Factory creates PRs but the pipeline ends there. When GitHub CI fails, no one automatically closes the loop — a human must notice, diagnose, and fix. For projects like Ising, this is the primary bottleneck: the agent that built the PR is already gone by the time CI reports failure.

**The gap:**
```
Factory: BUILD → STATIC GATE → INSPECT → PR Created → CI fails → ??? → human intervenes
```

**The goal:**
```
Factory: BUILD → STATIC GATE → INSPECT → PR Created → CI fails → extract error → auto-fix → push → CI re-runs → (repeat ≤ 3)
```

This is distinct from the existing STATIC GATE (which runs locally before PR creation). This covers **remote CI failures** — environment differences, missing deps, platform-specific issues, integration test failures that only surface in CI.

## Design

### Architecture: GitHub Action + Claude Code

The feedback loop is a **GitHub Action workflow** that triggers on CI check failure for agent-created PRs.

```
┌─────────────┐     ┌──────────────┐     ┌─────────────┐
│  CI fails   │────▶│  ci-fix.yml  │────▶│ Claude Code │
│ (check_run) │     │  (workflow)  │     │  (fix agent) │
└─────────────┘     └──────────────┘     └─────┬───────┘
                                               │
                                    ┌──────────▼──────────┐
                                    │ commit fix + push   │
                                    │ to same branch      │
                                    └──────────┬──────────┘
                                               │
                                    ┌──────────▼──────────┐
                                    │ CI re-triggers      │
                                    │ automatically       │
                                    └─────────────────────┘
```

### Trigger Conditions

The workflow fires when:
1. A `check_suite` completes with `conclusion: failure`
2. The PR branch matches a known agent pattern (e.g., `factory/*`, `claude/*`)
3. A `ci-fix-attempt` label count is < max attempts (3)

### Error Extraction

The fix agent needs actionable errors, not raw logs. The workflow:
1. Fetches failed check run logs via `gh api`
2. Extracts the failing step's output (build errors, test failures)
3. Truncates to last 200 lines of relevant output (avoid context bloat)
4. Passes structured error context to the fix agent

### Fix Agent Prompt

The fix agent receives:
- The original PR description (which includes the spec reference)
- The CI failure log (extracted + truncated)
- The list of files changed in the PR
- Instructions: fix the CI failure, commit, push to the same branch

The agent does NOT re-run the full factory pipeline — it makes a targeted fix.

### Iteration Cap

- Max 3 fix attempts per PR (tracked via GitHub labels: `ci-fix-attempt-1`, `ci-fix-attempt-2`, `ci-fix-attempt-3`)
- On reaching the cap: add `ci-fix-exhausted` label + comment explaining the failures
- Each attempt's fix is a separate commit for easy bisection

### Scope Control

The fix agent is constrained:
- Only modifies files already in the PR diff (no scope creep)
- Only addresses the specific CI failure (not general improvements)
- If the fix requires changes outside the PR's scope, it comments and stops

### Portability

The workflow is designed to be **portable across projects**:
- A reusable workflow (`.github/workflows/ci-fix.yml`) that any repo can adopt
- Configuration via repo variables: max attempts, branch patterns, CI job names
- The Ising project would add this workflow + configure it for its CI

## Plan

- [x] Design the `ci-fix.yml` reusable GitHub Action workflow
- [x] Implement error extraction logic (parse CI logs → actionable error summary)
- [x] Define the fix agent prompt template (spec context + CI error + constraints)
- [x] Add iteration tracking via GitHub labels
- [x] Add exhaustion handling (label + comment when max attempts reached)
- [x] Create a portable template that projects like Ising can adopt
- [x] Add governance logging — record CI fix attempts in `.harness/ci-fix.governance.jsonl`
- [x] Extend factory SKILL.md to document the post-PR CI feedback loop

## Test

- [ ] Simulate CI failure on a factory-created PR → verify workflow triggers
- [ ] Verify error extraction produces actionable output (not raw noise)
- [ ] Verify fix agent can resolve a simple CI failure (e.g., missing dependency)
- [ ] Verify iteration cap prevents infinite loops (stops at 3)
- [ ] Verify scope control — agent doesn't modify files outside PR diff
- [ ] Test on Ising project as first real-world validation

## Notes

### Why GitHub Action, not polling?

- **Event-driven** vs polling: no wasted compute, instant reaction
- **Runs where CI runs**: same environment, same permissions
- **Native GitHub integration**: labels, comments, check runs — no external orchestration
- **Portable**: any GitHub repo can adopt the workflow file

### Relationship to existing STATIC GATE

STATIC GATE catches local issues before PR creation (cargo check, clippy, etc.). The CI feedback loop catches **remote-only** failures:
- Platform differences (Ubuntu CI vs local macOS/container)
- Missing system dependencies not installed locally
- Integration tests that require CI-specific fixtures
- Flaky tests that only manifest in CI

These are complementary, not overlapping. A strong STATIC GATE reduces the CI feedback loop's workload.

### Alternative considered: Extend factory skill directly

Could add a "Step 5.5 — Monitor CI" to factory. Rejected because:
1. Factory session may already be closed/terminated
2. CI can take 10+ minutes — holding a session open is wasteful
3. GitHub Actions are the natural execution environment for CI-triggered work
4. Decoupled design: factory doesn't need to know about CI specifics

### Claude Code GitHub Action

This design assumes availability of a Claude Code GitHub Action (e.g., `anthropics/claude-code-action`) or equivalent mechanism to run Claude in a GitHub Actions context. If not available, the workflow would use the Claude API directly with a script.

### Superseded by 058

This spec's approach (GitHub Action dispatching a new amnesiac agent) was rejected after analysis. The fire-and-forget delegation pattern — spawning a new agent with no context about why the code was written — is fundamentally unreliable. Industry research (OpenAI harness engineering, Elastic self-healing CI, Dagger, OpenDev arXiv paper) confirms that coding agents need deterministic code-based orchestration, not prompt-based pipelines.

Spec 058 replaces this with a code-based harness where:
- CI monitoring is a pipeline step in the Rust CLI, not a separate GitHub Action
- The same agent session handles CI failures (context preserved via `--session-id`)
- A pre-PR CI Gate catches ~80% of failures before they reach GitHub
- Retry logic is enforced in code, not suggested in prompts

The `ci-fix.yml` workflow and `CI_FIX_ADOPTION.md` created by this spec should be removed as part of 058 implementation.
