# Factory Design

The factory skill implements a two-station assembly line for spec-driven development: **BUILD** (implement + test + commit) → **INSPECT** (adversarial review with fresh context).

## Core Thesis

A single AI agent can write code, but it can't objectively review its own output. The factory separates building from inspection — the INSPECT agent has fresh context with no visibility into the BUILD agent's reasoning. This adversarial independence catches bugs, spec violations, and quality issues that self-review misses.

## Pipeline Overview

```
/factory run <spec-path>

Orchestrator (main conversation)
  │
  ├─ Step 1: Initialize
  │    • Generate work ID (factory-{timestamp})
  │    • Create manifest at .factory/{work-id}/manifest.json
  │
  ├─ Step 2: BUILD (worktree-isolated subagent)
  │    • Read spec, implement code
  │    • Run tests, commit to branch
  │    • Output: BUILD REPORT (files, tests, commit, branch)
  │
  ├─ Step 2.5: STATIC GATE (no AI cost)
  │    • Language-specific: cargo check/clippy, tsc/eslint, pyright/ruff
  │    • Custom rules from .harness/rules/
  │    • Fail → route back to BUILD (max 2 retries)
  │    • Pass → proceed to INSPECT
  │
  ├─ Step 3: INSPECT (adversarial subagent, fresh context)
  │    • Review diff against spec requirements
  │    • Five dimensions: completeness, correctness, security, conformance, quality
  │    • Output: INSPECT VERDICT (APPROVE or REWORK with categorized items)
  │
  ├─ Step 4: Route
  │    • APPROVE → create PR
  │    • REWORK + attempts < 3 → back to BUILD with feedback
  │    • REWORK + attempts >= 3 → escalate to human
  │
  ├─ Step 5: Create PR (via gh CLI)
  │    • Push branch, create PR with build report
  │
  ├─ Step 6: Escalate (if 3 attempts exhausted)
  │    • Report failure, leave branch for manual review
  │
  └─ Step 7: Finalize
       • Update manifest with metrics
       • Append to GovernanceLog (.harness/factory.governance.jsonl)
```

## Key Properties

- **Isolation**: BUILD runs in a git worktree — can't pollute the main branch
- **Adversarial**: INSPECT has fresh context, never sees BUILD's reasoning
- **Bounded**: max 3 INSPECT rework cycles, max 2 static gate retries
- **Governed**: every cycle logged to `.harness/factory.governance.jsonl`
- **Zero-cost gates**: static checks (linters, type checkers) run before AI review

## Review Dimensions

INSPECT evaluates changes against five categories. Each rework item is tagged with its category for pattern tracking:

| Category | What it checks |
|----------|---------------|
| **completeness** | All spec plan items addressed |
| **correctness** | Logic bugs, off-by-one errors, flawed algorithms |
| **security** | Injection risks, hardcoded secrets, unsafe operations |
| **conformance** | Implementation matches spec intent, not just "something that works" |
| **quality** | Clean code, good structure, maintainability |

## Metrics

Each factory run records:

- **cycle_time_seconds**: wall-clock time from start to finish
- **total_attempts**: number of BUILD → INSPECT cycles
- **static_rework_count**: times BUILD was sent back by the static gate
- **first_pass_yield**: true if approved on attempt 1
- **rework_categories**: aggregated counts by category across all attempts

## Structured Output

BUILD and INSPECT use delimited blocks for reliable parsing:

```
=== BUILD REPORT ===
FILES: src/main.rs, src/lib.rs
TESTS: PASS
COMMIT: abc1234
BRANCH: factory/factory-1710600000
=== END BUILD REPORT ===
```

```
=== INSPECT VERDICT ===
VERDICT: REWORK
ITEMS:
- [completeness] Missing error handler for invalid input
- [correctness] Off-by-one in pagination logic
SUMMARY: Two issues need addressing before approval
=== END INSPECT VERDICT ===
```

## Crystallization (Future)

When the GovernanceLog accumulates enough data (10+ runs), a crystallization process can:

1. Identify high-frequency rework patterns (e.g., "completeness/missing-error-handler" in >30% of runs)
2. Generate static gate rules in `.harness/rules/` that catch these patterns before INSPECT
3. Create a feedback loop: more factory runs → fewer issues reach INSPECT

## References

- [SKILL.md](../../skills/factory/SKILL.md) — Full orchestration protocol with exact prompts
- [HARNESS.md](../../HARNESS.md) — Governance protocol and feedback taxonomy
- [evaluation-strategy.md](../evaluation/evaluation-strategy.md) — Benchmarks for measuring factory effectiveness
