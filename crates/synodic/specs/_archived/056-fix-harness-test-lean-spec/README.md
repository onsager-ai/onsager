---
status: planned
created: 2026-03-22
priority: critical
tags:
- harness
- bugfix
- lean-spec
created_at: 2026-03-22T12:50:44.604920400Z
updated_at: 2026-03-22T12:50:44.604920400Z
---

# Fix Harness Critical Gaps Found in lean-spec Real-World Test

## Overview

Spec 055 ran the harness against codervisor/lean-spec (real polyglot monorepo) and found 8 critical gaps that make governance unusable on real projects. The agent produced correct code on attempt 1, but the harness rejected it 3 times with zero actionable feedback — wasting 18 minutes of compute. This spec fixes the 5 most critical bugs that fire on every real-world run.

## Design

### Fix 1: Static gate stdout parsing (F1 — feedback loop broken)

**Problem:** `static_gate.sh` outputs log lines before JSON. Harness parses ALL stdout as JSON → fails silently → empty feedback.

**Fix:** In `run.rs:142-147`, parse stdout line-by-line looking for the first valid JSON object instead of parsing the entire output. Also capture stderr as fallback feedback text when JSON parsing fails entirely.

```rust
// Try to find JSON in output (may be preceded by log lines)
let gate_out = String::from_utf8_lossy(&output.stdout);
let mut parsed = false;
for line in gate_out.lines() {
    let trimmed = line.trim();
    if trimmed.starts_with('{') {
        if let Ok(report) = serde_json::from_str::<Value>(trimmed) {
            if let Some(failures) = report.get("failures").and_then(|v| v.as_array()) {
                l1_failures.extend(failures.iter().cloned());
                parsed = true;
            }
            break;
        }
    }
}
// Fallback: use raw stderr + stdout as feedback text
if !parsed {
    let stderr_text = String::from_utf8_lossy(&output.stderr);
    let raw_feedback = if !stderr_text.trim().is_empty() {
        stderr_text.to_string()
    } else {
        gate_out.to_string()
    };
    l1_failures.push(json!({
        "checker": "static_gate",
        "output": raw_feedback.lines().take(50).collect::<Vec<_>>().join("\n")
    }));
}
```

### Fix 2: Baseline test isolation (F2 — preexisting failures block correct code)

**Problem:** Gate runs ALL tests. Preexisting failures → agent's correct code rejected.

**Fix:** Before the governance loop starts, run the static gate at `base_ref` to capture baseline failures. Then in the loop, only fail on NEW failures not present in the baseline.

```rust
// Before loop: capture baseline failures
let baseline_failures: HashSet<String> = if static_gate.exists() {
    // git stash, checkout base_ref, run gate, restore
    run_baseline_gate(&static_gate, &workdir, &base_ref)
};
// In loop: filter out baseline failures
let new_failures: Vec<_> = l1_failures.iter()
    .filter(|f| !baseline_failures.contains(failure_key(f)))
    .collect();
```

### Fix 3: L2 judge fails-open → fails-closed (F4 — inverted safety)

**Problem:** `run.rs:291-293` and `run.rs:378` auto-approve when judge fails or verdict is unparseable.

**Fix:** Change default to ESCALATE when judge fails, with explicit `--judge-fail-open` flag for opt-in.

```rust
// Line 291: judge process failure
if judge_exit != 0 {
    log_info(&config, "AI judge failed. Escalating (fail-closed).");
    status = "escalated".to_string();
    break;
}
// Line 378: unparseable verdict
} else {
    log_info(&config, "AI judge: could not parse verdict. Escalating.");
    status = "escalated".to_string();
    break;
}
```

Add `pub judge_fail_open: bool` to `RunConfig` and gate the old behavior behind it.

### Fix 4: Agent and judge timeouts (F7 — can hang forever)

**Problem:** `Command::new(&cmd[0]).status()` has no timeout. Hanging agent/judge blocks forever.

**Fix:** Add configurable timeouts with `wait_timeout` pattern.

```rust
pub agent_timeout_s: u64,   // default 1800 (30 min)
pub judge_timeout_s: u64,   // default 300 (5 min)
```

Use `child.wait_timeout()` or spawn a watchdog thread that kills the child after the deadline.

### Fix 5: Exclude harness artifacts from diff (F8 — polluted observation)

**Problem:** `.harness/.runs/` artifacts included in `git diff`, polluting L1 and L2 review.

**Fix:** Add `-- ':!.harness'` pathspec exclusion to all git diff commands in `observe_changes()`.

```rust
let diff_output = Command::new("git")
    .args(["-C", &wd, "diff", &diff_range, "--", ".", ":!.harness"])
    .output()?;
```

### Fix 6: Static gate execution contract (F3)

**Problem:** No `current_dir`, no timeout, no documented interface.

**Fix:** Set `.current_dir(workdir)` on the static gate Command (run.rs:132). Add env vars `HARNESS_BASE_REF`, `HARNESS_WORKDIR`. Add 10-minute timeout.

### Fix 7: Close stdin before wait (F6)

**Problem:** `run_agent_with_stdin` never explicitly closes stdin → agent may hang waiting for EOF.

**Fix:** Drop stdin handle explicitly before `child.wait()`.

```rust
if let Some(mut stdin) = child.stdin.take() {
    let _ = stdin.write_all(input.as_bytes());
    drop(stdin);  // explicit close — signals EOF to agent
}
```

## Plan

- [ ] Fix static gate stdout parsing — line-by-line JSON search + stderr fallback (F1)
- [ ] Add baseline test isolation — run gate at base_ref, diff failures (F2)
- [ ] Change judge failure mode to fail-closed with opt-in flag (F4)
- [ ] Add agent timeout (default 30m) and judge timeout (default 5m) (F7)
- [ ] Exclude `.harness/` from diff observation (F8)
- [ ] Set `current_dir` + env vars on static gate Command (F3)
- [ ] Close stdin explicitly before `child.wait()` (F6)
- [ ] Add unit tests for new parsing, baseline diff, and timeout logic

## Test

- [ ] Static gate with mixed text+JSON stdout produces correct `l1_failures`
- [ ] Preexisting test failures in base_ref do NOT cause L1 rejection
- [ ] Judge subprocess failure → status "escalated" (not "passed")
- [ ] Agent hanging beyond timeout → killed + escalated
- [ ] `git diff` output excludes `.harness/` directory
- [ ] Static gate runs with correct `current_dir` and env vars
- [ ] stdin closed before wait — agent receives EOF signal
- [ ] Re-run harness against lean-spec project — correct code passes on attempt 1

## Notes

- Findings sourced from spec 055 (harness real-world assessment against lean-spec @ c55cf343)
- F5 (nested Claude env) is environmental — not fixable in harness code, but fail-closed (F4 fix) prevents silent auto-approve
- Baseline isolation (F2) is the highest-impact fix — most real projects have some preexisting test noise
- Consider adding `.harness/` to `.gitignore` as an alternative/complement to F8