---
status: archived
created: 2026-03-22
priority: medium
tags:
- testing
- harness
- lean-spec
created_at: 2026-03-22T09:19:52.428507463Z
updated_at: 2026-03-22T11:27:06.442745708Z
transitions:
- status: archived
  at: 2026-03-22T11:27:06.442745708Z
---

# Synodic Harness Test Against lean-spec Project

## Overview

End-to-end validation of synodic's harness governance pipeline against the `codervisor/lean-spec` external project. Tests L1 (static rules), L2 (AI judge), rework loop, and governance logging.

## Test Results

### Test 1: L1 Pass (no static gate, --no-l2)
- **Status:** PASSED
- Agent made a clean change (13-line JS utility function)
- No static gate script present → L1 auto-pass
- Governance log recorded correctly

### Test 2: L1 Reject with Static Gate + Escalation
- **Status:** ESCALATED (exit code 2)
- Custom `static_gate.sh` checked for `console.log` in JS files
- Agent introduced `console.log` → L1 rejected
- Rework loop triggered (max-rework=1) → agent couldn't fix → escalated to human
- Rework feedback file generated at `.harness/.runs/{id}/feedback.md`

### Test 3: L1 Pass + L2 AI Judge
- **Status:** PASSED
- L1 passed (no static gate for clean change)
- L2 invoked `claude` as judge with diff context
- Judge output couldn't be parsed for `VERDICT:` pattern → accepted by default
- Raw judge output saved to `.harness/.runs/{id}/judge-attempt-1.log`
- Duration: 30s (mostly L2 judge latency)

## Findings

- [x] Harness builds and runs correctly against external repos
- [x] L1 static gate integration works (custom scripts)
- [x] Rework loop and escalation logic works correctly
- [x] Governance JSONL logging works
- [x] `harness log` and `harness log --json` display correctly
- [x] `--dry-run` mode works
- [x] `--json` output mode works
- [x] Exit codes correct: 0=passed, 1=error, 2=escalated

## Issues Found

1. **L2 judge verdict parsing:** When `claude` is invoked as judge, it doesn't produce `VERDICT: APPROVE` format by default — the harness accepts by default when it can't parse. The judge prompt may need stronger formatting instructions.
2. **Static gate discovery:** The harness looks for `static_gate.sh` in the synodic repo's `.harness/` rather than the target project's `.harness/` when `SYNODIC_ROOT` is set — workdir resolution should prefer target project.

## Notes

- All 35 synodic tests pass
- Tested on lean-spec commit `c55cf343` (main branch)
- git commit signing must be disabled for test scripts (`commit.gpgsign false`)
