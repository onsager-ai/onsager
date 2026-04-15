---
status: draft
created: 2026-03-28
priority: critical
tags:
- architecture
- governance
- hooks
- ci
- l1
- l2
created_at: 2026-03-28T00:00:00Z
updated_at: 2026-03-28T00:00:00Z
---

# Governance Architecture v2 — Hooks + CI as L1, Synodic as L2

## Overview

Redesign Synodic's governance architecture around a key insight: **git hooks and CI are the real L1**. Synodic should not duplicate what the ecosystem already provides. Instead, Synodic's value is L2 (AI judge), event collection, rework loops, and pattern visibility.

This spec supersedes the L1 infrastructure previously built into `harness-core` (l1.rs, PatternTracker, crystallized rules, static_gate.sh) — all of which has been removed.

## Problem

The previous architecture had four overlapping layers doing the same work:

1. Crystallized rules in `.harness/rules/` (custom format, custom runner)
2. `static_gate.sh` script (custom shell runner)
3. Gates in `gates.yml` (custom gate system)
4. Git hooks (industry standard)

This violated the project's North Star principles:
- **Keep things simple** — four layers for one job is not simple
- **Don't over-engineer** — custom L1 infrastructure when git hooks exist
- **Don't create duplication** — every layer ran the same checks (fmt, clippy, test)

## Design

### Layer 1: Git Hooks + CI

L1 is **not Synodic's job**. L1 belongs to the tools that already do it well:

| Mechanism | Scope | Enforceability | Speed |
|-----------|-------|----------------|-------|
| **pre-commit hook** | Local, per-commit | Advisory (can be bypassed) | Fast (<1s) |
| **pre-push hook** | Local, per-push | Advisory (can be bypassed) | Medium (seconds) |
| **CI (GitHub Actions)** | Remote, per-PR | **Enforced** (branch protection) | Slow (minutes) |

**Git hooks** catch issues early and fast. They work for every developer and every AI agent because everyone commits through git.

**CI** is the only gate you can actually enforce. Hooks are local convenience; CI is the authority.

#### Hook setup

Synodic provides hooks in `.githooks/` and activates them via:

```json
{
  "scripts": {
    "prepare": "git config core.hooksPath .githooks"
  }
}
```

This is zero-setup: `pnpm install` (or `npm install`) activates hooks automatically. No extra tooling required.

For projects already using Husky, Synodic does not replace it. Synodic builds **on top of** existing hook infrastructure by providing the feedback loop that hooks alone cannot:

- Event collection from hook failures
- Pattern detection across repeated failures
- L2 semantic review of what hooks can't catch

#### Current hooks

**pre-commit** — runs when `.rs` files are staged:
- `cargo fmt --all -- --check`

**pre-push** — runs when `.rs` files changed vs remote:
- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`

### Layer 2: Synodic (AI Judge + Event Governance)

This is where Synodic adds value that no other tool provides:

#### What L2 does

1. **AI Judge** — independent LLM reviews agent output with fresh context. Catches semantic issues that no regex or linter can find:
   - Agent actions that diverge from user intent (misalignment)
   - Hallucinated file/API references that look syntactically valid
   - Compliance violations that require understanding context
   - Subtle security issues that pass static analysis

2. **Event Collection** — structured capture of governance-relevant events from agent sessions:
   - Tool call errors
   - Hallucinations
   - Compliance violations
   - Misalignment signals

3. **Pattern Detection** — rules engine that matches patterns in event streams:
   - Regex-based detection (secrets, dangerous commands, force pushes)
   - Source-specific parsers (Claude Code JSONL, Copilot events)
   - Severity classification and routing

4. **Rework Loop** — the governance loop's real value. When L2 finds issues:
   - Structured feedback is generated
   - Agent receives specific, actionable correction
   - Loop continues until pass or escalation
   - All attempts are logged for audit

5. **Dashboard + Visibility** — surfaces patterns that individual hooks/CI runs can't show:
   - Event feed across sessions
   - Resolution queue for human review
   - Analytics on recurring governance issues
   - Rules management

#### What L2 does NOT do

- Run linters (that's hooks/CI)
- Run tests (that's hooks/CI)
- Check formatting (that's hooks/CI)
- Enforce branch protection (that's GitHub)
- Replace Husky or similar tools

### Architecture Diagram

```
Developer / AI Agent
        │
        ├── git commit ──► pre-commit hook (fmt check) ──► fast feedback
        │
        ├── git push ──► pre-push hook (fmt + clippy + test) ──► local feedback
        │
        ├── push to remote ──► CI (GitHub Actions) ──► ENFORCED gate
        │
        └── session complete ──► Synodic L2
                                    ├── AI judge (semantic review)
                                    ├── Event collection
                                    ├── Pattern detection
                                    ├── Rework loop (if issues found)
                                    └── Dashboard (visibility)
```

### Integration with Orchestra Pipelines

The orchestra pipeline engine (now merged back into synodic) uses the governance system at key checkpoints:

- **Factory pipeline**: L2 review after BUILD step, before PR creation
- **Adversarial pipeline**: L2 review validates attack/defense rounds
- **Fractal pipeline**: L2 review after reunification
- **Swarm pipeline**: L2 review before final merge

Pipeline `gates.yml` controls preflight checks for pipeline steps — this is separate from L1 governance and remains as-is.

## What Changed

### Removed (L1 infrastructure)

| Component | File | Reason |
|-----------|------|--------|
| `L1Evaluator` | `harness-core/src/l1.rs` | Dead code, duplicated hooks |
| `PatternTracker` | `harness-core/src/rules/mod.rs` | Crystallization replaced by hooks |
| `promotion_candidates()` | `harness-core/src/rules/mod.rs` | No longer needed |
| `static_gate.sh` | `.harness/scripts/static_gate.sh` | Replaced by git hooks |
| Crystallized rules | `.harness/rules/` | Replaced by git hooks |
| L1 section in run.rs | `harness-cli/src/harness/run.rs` | ~100 lines removed |
| `is_executable()` | `harness-cli/src/harness/run.rs` | Only used by L1 section |
| Rules/scripts dir init | `harness-cli/src/cmd/init.rs` | No longer created |

### Kept (still valuable)

| Component | File | Reason |
|-----------|------|--------|
| `Rule`, `RuleEngine` | `harness-core/src/rules/mod.rs` | Pattern detection in event streams |
| `default_rules()` | `harness-core/src/rules/mod.rs` | Regex rules for secrets, dangerous cmds |
| `evaluate()`, `matches_to_events()` | `harness-core/src/rules/mod.rs` | Core detection logic |
| L2 AI judge | `harness-cli/src/harness/run.rs` | Semantic review — unique value |
| Event types + storage | `harness-core/src/events.rs`, `storage/` | Structured event capture |
| Log parsers | `harness-core/src/parsers/` | Agent log analysis |

## Future Considerations

### Husky Integration

For projects that already use Husky, Synodic could provide a plugin or configuration generator that:
- Adds governance-aware hooks alongside existing Husky hooks
- Collects hook failure events into Synodic's event store
- Reports hook failure patterns in the dashboard

This is a future enhancement, not a current requirement. The `.githooks/` + `prepare` script approach works today with zero dependencies.

### Hook Event Collection

A future enhancement could have hooks report failures as Synodic events:

```bash
# In pre-push hook, on failure:
synodic submit --type compliance_violation \
  --title "Pre-push check failed: clippy" \
  --severity medium \
  --metadata '{"hook": "pre-push", "check": "clippy"}'
```

This would close the loop between L1 (hooks) and L2 (Synodic), giving the dashboard visibility into local check failures — not just CI failures.

## Success Criteria

- [ ] No custom L1 infrastructure in Synodic codebase (verified: removed)
- [ ] Git hooks in `.githooks/` catch formatting, lint, and test issues locally
- [ ] CI enforces the same checks as a hard gate on PRs
- [ ] L2 AI judge still functions for semantic review
- [ ] Rules engine still detects patterns in event streams
- [ ] Dashboard still surfaces governance events
- [ ] Orchestra pipelines still integrate with L2 at checkpoints

## Non-Goals

- Replacing Husky or any existing hook manager
- Running linters/tests inside Synodic
- Custom rule formats that duplicate what hooks/CI already check
- Making hooks un-bypassable (that's CI's job)
