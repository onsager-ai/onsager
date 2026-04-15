---
status: complete
created: 2026-04-03
priority: high
tags:
- pipeline
- governance
- inspect
- semantic
- L2
depends_on:
- "077"
- "069"
created_at: 2026-04-03T00:00:00Z
updated_at: 2026-04-03T00:00:00Z
---

# Semantic INSPECT — L2 QA Review in the Pipeline

> **Status**: draft · **Priority**: high · **Created**: 2026-04-03

## Overview

INSPECT currently only runs L1 deterministic checks — shell commands (`cargo fmt`, `cargo test`) that exit 0 or non-zero. This is CI, not QA. A real inspector reviews *what changed* and *whether it matches intent*.

BUILD invokes Claude as a developer. INSPECT should invoke Claude as a QA reviewer — an independent LLM session that reviews the diff against the task prompt, project rules, and semantic quality criteria. This is the L2 Audit quadrant from spec 069's capability matrix, applied inside the pipeline loop.

### What L1 checks miss

- **Goal drift** — agent wandered from the prompt, added unrelated changes
- **Scope creep** — agent "improved" code that wasn't part of the task
- **Security patterns** — hardcoded credentials, injection vectors, unsafe patterns that linters don't catch
- **Architectural violations** — bypassed abstractions, wrong layers, coupling
- **Missing coverage** — new code paths without corresponding tests
- **Hallucinations** — references to nonexistent APIs, functions, or packages

These require understanding intent and context. Only an LLM can judge them.

## Design

### Check types in `pipeline.yml`

Checks gain an explicit `type` field. L1 and L2 checks have different schemas:

```yaml
checks:
  # L1 — deterministic (default, backward-compatible)
  - name: format
    run: "cargo fmt --all -- --check"
    fix: "cargo fmt --all"
    stage: commit

  - name: test
    run: "cargo test"

  # L2 — semantic review (new)
  - name: security-review
    type: semantic
    prompt: "Review for security vulnerabilities: secrets, injection, unsafe patterns"
    severity: block          # block = hard fail, warn = report but pass

  - name: goal-alignment
    type: semantic
    prompt: "Does the diff match the task? Flag scope creep, unrelated changes, goal drift"
    severity: block
```

**Type field:**
- `run` (default, omitted) — L1 deterministic. Has `run`, `fix`, `stage` fields.
- `semantic` — L2 LLM review. Has `prompt`, `severity` fields. No `run`/`fix`/`stage`.

**Severity field:**
- `block` (default) — failure triggers rework, same as L1 check failure
- `warn` — findings reported in UI but don't block the pipeline

### Execution order

```
INSPECT
  1. L1 checks (format, lint, test)     — fast, cheap, deterministic
  2. L2 checks (security, alignment)    — only if L1 passes
```

L2 runs *after* L1 passes. No point paying for an LLM review of code that doesn't compile.

### L2 check execution

Each semantic check invokes Claude with:

```
System: You are a QA reviewer. Review the diff below against the given criteria.
        Return a structured JSON verdict.

Context:
  - Task prompt: {original prompt}
  - Diff: {git diff output}
  - Check: {check name}
  - Criteria: {check prompt from pipeline.yml}

Output format:
  { "passed": true/false, "findings": ["...", "..."] }
```

Implementation options:
1. **`claude --print`** — same as BUILD, parse stream-json for result
2. **Direct API call** — `reqwest` to Anthropic API, avoids spawning a process per check

Option 2 is better for L2 checks: faster, cheaper (no tool overhead), structured output via tool_use response format.

### L2 check result → `CheckResult`

Semantic checks produce the same `CheckResult` that L1 checks do:

```rust
CheckResult {
    name: "security-review",
    passed: false,                    // or true
    exit_code: 0,                     // always 0 (not a process)
    stdout: "Found 2 issues:\n...",   // LLM findings as text
    stderr: "",
    duration_ms: 3200,
}
```

This means rework feedback works identically — the BUILD agent gets the LLM reviewer's findings as rework context, just like it gets `cargo test` output.

### Wire `fix` for L1 checks

While adding L2, also wire the existing `fix` field for L1 checks. Currently parsed but never used.

When an L1 check fails and has `fix`:
1. Run the fix command
2. Re-run the check
3. If it passes now, auto-commit the fix and continue
4. If it still fails, report as normal failure

This handles `cargo fmt` automatically without burning a BUILD rework cycle.

### UI integration

L2 checks get the same spinner treatment as L1:

```
INSPECT
  ✓ format                                (0.1s)
  ✓ lint                                  (0.9s)
  ✓ test                                  (1.1s)
  ⠋ security-review...
  ✓ security-review                       (3.2s)
  ⠋ goal-alignment...
  ✗ goal-alignment                        (2.8s)
      Scope creep: modified logging.rs which is unrelated to the task
      Added TODO comments not requested in the prompt
```

### Config type changes

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Check {
    /// L1 deterministic check (default when `type` omitted)
    #[serde(rename = "run", alias = "")]
    Run {
        name: String,
        run: String,
        fix: Option<String>,
        stage: Option<Stage>,
    },
    /// L2 semantic review
    Semantic {
        name: String,
        prompt: String,
        #[serde(default = "default_severity")]
        severity: Severity,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Block,
    Warn,
}
```

Backward compatibility: checks without a `type` field default to `run` type. Existing `pipeline.yml` files keep working.

## Non-goals

- **Multi-model review** — using different models for different L2 checks (future)
- **Custom LLM endpoints** — only Anthropic API for now
- **Consensus review** — running the same check N times and voting (future)
- **Interactive review** — L2 reviewer asking the BUILD agent questions (future)

## Plan

### Phase 1: Check type refactor

- [ ] Refactor `Check` from flat struct to tagged enum (`Run` / `Semantic`)
- [ ] Add `Severity` enum
- [ ] Update config parsing — default type to `run` for backward compat
- [ ] Update all code that accesses `Check` fields (runner, generator, tests)
- [ ] Existing tests still pass

### Phase 2: Wire L1 `fix` field

- [ ] When L1 check fails and has `fix`: run fix command, re-check
- [ ] Auto-commit fix if re-check passes
- [ ] UI shows "auto-fixed" indicator
- [ ] Skip rework cycle for auto-fixable failures

### Phase 3: L2 semantic check execution

- [ ] Implement `run_semantic_check()` — calls Anthropic API with diff + prompt
- [ ] Structured JSON response → `CheckResult`
- [ ] `run_checks_ui()` handles both L1 and L2 checks
- [ ] L2 runs only after all L1 pass
- [ ] Severity `warn` vs `block` routing

### Phase 4: Default semantic checks

- [ ] Ship default `security-review` and `goal-alignment` checks in `synodic init`
- [ ] Tuned prompts for each language (Rust, Node, Python, Go)
- [ ] `--no-semantic` flag to skip L2 checks (for speed/cost control)

## Test

- [ ] Existing `pipeline.yml` without `type` field parses correctly (backward compat)
- [ ] `type: semantic` check parsed with `prompt` and `severity`
- [ ] L1 check with `fix` auto-repairs and re-checks
- [ ] L2 check produces `CheckResult` with findings
- [ ] L2 `severity: warn` does not trigger rework
- [ ] L2 `severity: block` triggers rework with findings in feedback
- [ ] L2 checks skipped when L1 fails (no wasted API calls)
- [ ] `--no-semantic` flag skips all L2 checks
