---
status: complete
created: 2026-04-03
priority: high
tags:
- governance
- telemetry
- pipeline
- feedback
depends_on:
- "072"
- "073"
- "077"
created_at: 2026-04-03T00:00:00Z
updated_at: 2026-04-03T00:00:00Z
---

# Pipeline Telemetry — Wire `synodic run` Events to Governance

> **Status**: draft · **Priority**: high · **Created**: 2026-04-03

## Overview

`synodic run` executes the Build->Inspect->PR pipeline but currently discards all telemetry. Check results, build duration, API cost, and pipeline outcomes exist in memory during the run and vanish when the process exits. This spec wires pipeline events to the governance storage layer so every run feeds the scoring engine (074) and rule lifecycle (076).

This is the connection between "run the pipeline" (077) and "governance that learns" (072-076). Without it, the governance engine has no data.

## Design

### Event types

Every `synodic run` produces these events:

| Event | Storage | Feeds |
|-------|---------|-------|
| Check passed | `feedback_events` (signal: `ci_pass`) | F(R) friction score, coverage stats |
| Check failed | `feedback_events` (signal: `ci_failure`) | F(R) friction, S(R) safety, rule candidates |
| Build complete | `pipeline_runs` (new table) | Cost tracking, duration trends |
| Pipeline passed | `pipeline_runs` | Success rate |
| Pipeline failed | `pipeline_runs` | Failure patterns |

### `pipeline_runs` table (new)

```sql
CREATE TABLE pipeline_runs (
    id TEXT PRIMARY KEY,
    prompt TEXT NOT NULL,
    branch TEXT,
    outcome TEXT NOT NULL,          -- "passed" | "failed" | "error"
    attempts INTEGER NOT NULL,
    model TEXT,
    build_duration_ms INTEGER,
    build_cost_usd REAL,
    inspect_duration_ms INTEGER,
    total_duration_ms INTEGER NOT NULL,
    project_id TEXT,
    created_at TEXT NOT NULL
);

CREATE INDEX idx_pipeline_runs_created ON pipeline_runs(created_at);
CREATE INDEX idx_pipeline_runs_outcome ON pipeline_runs(outcome);
```

### Check result events

For each check in INSPECT, record a feedback event:

```rust
store.record_feedback(FeedbackEvent {
    signal_type: if result.passed { "ci_pass" } else { "ci_failure" },
    rule_id: format!("ci-{}", result.name),  // e.g. "ci-format", "ci-test"
    tool_name: "synodic-run",
    tool_input: json!({
        "check": result.name,
        "command": check.run,
        "exit_code": result.exit_code,
        "duration_ms": result.duration_ms,
    }),
    ..
})
```

This reuses the existing `feedback_events` table and `record_feedback` method from spec 072. The `ci_pass` signal is new (073 defined `ci_failure` but not pass) — adding it is a one-line change since `record_feedback` accepts any string.

### Wiring into `run_pipeline`

The `run_pipeline` function gains an optional `Storage` parameter:

```rust
pub async fn run_pipeline(
    config: &PipelineConfig,
    run_cfg: &RunConfig,
    ui: &PipelineUi,
    store: Option<&dyn Storage>,  // new — None = no telemetry
) -> Result<RunOutcome>
```

When `store` is `Some`:
- After each INSPECT check: record `ci_pass` or `ci_failure`
- After BUILD: record build duration and cost (from stream-json result event)
- After pipeline completes: record `pipeline_runs` entry

When `store` is `None`: pipeline works identically, just no telemetry. This keeps `synodic run` usable without a database.

### CLI integration

`cmd/run.rs` optionally opens the DB:

```rust
let store = match storage::pool::try_create_storage() {
    Ok(s) => Some(s),
    Err(_) => {
        // No DB configured — run without telemetry
        None
    }
};
```

No `--db-url` flag needed — uses the same `DATABASE_URL` env var / default SQLite path as other commands.

### What this enables

With telemetry flowing, existing commands gain real data:

- `synodic status` — F(R) friction score now reflects actual check pass/fail rates from pipeline runs
- `synodic rules optimize` — can propose rule candidates from recurring `ci_failure` patterns
- `synodic rules check` — auto-transitions based on accumulated evidence
- Dashboard (future) — pipeline run history, cost trends, success rates

## Non-goals

- Real-time event streaming to external systems (that's spec 077 Phase 6)
- Pipeline run scheduling/cron (separate concern)
- Detailed token-level cost breakdown (just total cost from Claude's result event)

## Plan

### Phase 1: Storage additions

- [ ] Add `pipeline_runs` table to migration
- [ ] Add `record_pipeline_run()` and `get_pipeline_runs()` to Storage trait
- [ ] Implement in SQLite storage
- [ ] Add `ci_pass` as recognized signal type alongside `ci_failure`

### Phase 2: Wire telemetry into pipeline

- [ ] Add `store: Option<&dyn Storage>` param to `run_pipeline` and `run_pipeline_loop`
- [ ] Record check results as feedback events after each INSPECT
- [ ] Capture build cost/duration from stream-json result event
- [ ] Record pipeline run entry on completion
- [ ] Update `cmd/run.rs` to open storage and pass through

### Phase 3: Verify integration

- [ ] `synodic run --dry-run` records check events to DB
- [ ] `synodic status` reflects data from pipeline runs
- [ ] `synodic rules optimize` sees ci_failure events from pipeline

## Test

- [ ] `record_pipeline_run()` inserts and retrieves correctly
- [ ] Check pass records `ci_pass` feedback event with correct fields
- [ ] Check fail records `ci_failure` feedback event
- [ ] Pipeline with no DB configured runs without error
- [ ] `synodic status` scores change after pipeline run with failures
