---
status: archived
created: 2026-03-11
priority: high
tags:
- factory
- metrics
- observability
- throughput
- bottleneck
- dashboard
parent: 037-coding-factory-vision
depends_on:
- 039-assembly-line-abstraction
- 040-factory-quality-system
created_at: 2026-03-11T00:00:00Z
updated_at: 2026-03-11T00:00:00Z
---

# Production Metrics & Dashboard — Measuring the Factory

## Overview

A factory is measured, not felt. Individual agent health is spec'd (spec 004), but factory-level metrics are completely absent. This is like having a foreman who can check if a worker is alive, but no one measuring how many cars roll off the line per hour.

This spec builds the production observability layer: the instruments that measure throughput, cycle time, defect rate, cost per unit, and bottleneck position — and the dashboard that renders this data into actionable factory-floor visibility.

## Design

### Core Metrics

Nine factory metrics, measured continuously:

| Metric | Definition | Collection Point |
|---|---|---|
| **Throughput** | Work items completing all stations per time period | Line exit |
| **Cycle time** | Wall-clock time from first station entry to line exit | Per-item timestamps |
| **Lead time** | Time from work item creation to production deployment | INTAKE entry → DEPLOY exit |
| **Defect rate** | % of items requiring rework at any station | Quality gate verdicts |
| **First-pass yield** | % of items passing all stations without any rework | Work item history |
| **Cost per unit** | Total compute cost (tokens + API calls) per completed item | Per-station token counters |
| **Agent utilization** | % of time agents are productively working vs idle/waiting | Station activity tracking |
| **WIP count** | Work items currently in-flight across all stations | Conveyor state |
| **Station dwell time** | Average time a work item spends at each station | Per-station entry/exit timestamps |

### Collection Architecture

```
 ┌──────────┐  ┌──────────┐  ┌──────────┐
 │ Station 1│  │ Station 2│  │ Station N│
 └────┬─────┘  └────┬─────┘  └────┬─────┘
      │              │              │
      │ events       │ events       │ events
      ▼              ▼              ▼
 ┌─────────────────────────────────────────┐
 │          Metrics Collector              │
 │  (subscribe to conveyor + station       │
 │   events via message bus)               │
 └────────────────┬────────────────────────┘
                  │
                  ▼
 ┌─────────────────────────────────────────┐
 │          Metrics Store                  │
 │  (SQLite — extends spec 006 schema)     │
 │                                         │
 │  Tables:                                │
 │  - work_item_events (immutable log)     │
 │  - station_snapshots (periodic)         │
 │  - cost_ledger (per-item, per-station)  │
 │  - metric_aggregates (pre-computed)     │
 └────────────────┬────────────────────────┘
                  │
                  ▼
 ┌─────────────────────────────────────────┐
 │          Dashboard                      │
 │  (CLI + optional web UI)                │
 └─────────────────────────────────────────┘
```

### Events

Every factory activity emits a structured event:

```json
{
  "event": "station.work_item.completed",
  "timestamp": "2026-03-11T14:23:00Z",
  "work_item_id": "work-001",
  "station_id": "build",
  "duration_ms": 45000,
  "tokens_used": 12500,
  "quality_verdict": "pass",
  "rework": false
}
```

Event types:
- `work_item.created`, `work_item.completed`, `work_item.escalated`
- `station.work_item.entered`, `station.work_item.completed`, `station.work_item.rework`
- `station.agent.assigned`, `station.agent.idle`, `station.agent.error`
- `conveyor.backpressure.engaged`, `conveyor.backpressure.released`
- `andon.triggered`, `andon.resolved`

### Bottleneck Detection

Automated Theory of Constraints analysis:

1. **Identify the constraint:** The station with the highest average dwell time (or lowest throughput) is the bottleneck
2. **Exploit the constraint:** Ensure the bottleneck station has maximum agent staffing and no idle time
3. **Subordinate:** Upstream stations throttle to match bottleneck capacity (back-pressure already handles this)
4. **Elevate:** Alert when bottleneck dwell time exceeds 2x the line average — suggests the station needs more agents, a better coordination primitive, or task decomposition

Bottleneck analysis runs on a rolling window (last 50 work items or last hour, whichever is more).

### Cost Accounting

Per-item cost breakdown:

```json
{
  "work_item_id": "work-001",
  "total_cost_usd": 0.47,
  "breakdown": {
    "intake": { "tokens": 2100, "cost_usd": 0.02 },
    "design": { "tokens": 8500, "cost_usd": 0.08 },
    "build":  { "tokens": 45000, "cost_usd": 0.22 },
    "inspect": { "tokens": 12000, "cost_usd": 0.11 },
    "rework":  { "tokens": 4500, "cost_usd": 0.04 }
  },
  "model_tier_breakdown": {
    "frontier": { "tokens": 35000, "cost_usd": 0.38 },
    "mid":      { "tokens": 37100, "cost_usd": 0.09 }
  }
}
```

Cost per unit trends over time are critical for validating Nemosis (spec 016) effectiveness.

### Dashboard

Two output modes:

**CLI dashboard** (`synodic dashboard`):
```
╔═══════════════════════════════════════════════════════════════╗
║  SYNODIC FACTORY — LIVE                     2026-03-11 14:30 ║
╠═══════════════════════════════════════════════════════════════╣
║                                                               ║
║  Throughput: 12 units/hr    Cycle time: 8m (p50) 14m (p95)  ║
║  First-pass: 82%            Defect rate: 4.2%                ║
║  Cost/unit: $0.47           WIP: 7/20                        ║
║                                                               ║
║  INTAKE ██░░ (2)  →  DESIGN █░░░ (1)  →  BUILD ████ (4)     ║
║  →  INSPECT ██░░ (2)  →  HARDEN ░░░░ (0)  →  DEPLOY ░░░░   ║
║                                                               ║
║  ⚡ Bottleneck: BUILD (dwell 4.2m, 1.8x avg)                ║
║                                                               ║
╚═══════════════════════════════════════════════════════════════╝
```

**Web UI** (optional, Phase 2+): Real-time factory floor visualization with station graphs, trend lines, and drill-down per work item.

## Plan

- [ ] Define event schema for all factory activities (work item, station, conveyor, andon events)
- [ ] Implement metrics collector: subscribe to message bus events, compute running aggregates
- [ ] Extend SQLite schema (spec 006) with work_item_events, station_snapshots, cost_ledger tables
- [ ] Implement per-item cost tracking: token counting per station, per model tier
- [ ] Implement bottleneck detection: rolling window analysis, constraint identification, elevation alerts
- [ ] Compute aggregate metrics: throughput, cycle time percentiles, first-pass yield, defect rate
- [ ] Build CLI dashboard (`synodic dashboard`) with live-updating factory floor view
- [ ] Implement `synodic metrics <work-item-id>` for per-item drill-down
- [ ] Add historical trend queries: `synodic trends --period 7d` for week-over-week comparison

## Test

- [ ] Processing 10 work items through a 3-station line produces correct throughput and cycle time metrics
- [ ] Cost accounting tracks tokens per station and produces accurate per-item cost breakdowns
- [ ] Bottleneck detection correctly identifies the slowest station as the constraint
- [ ] First-pass yield correctly excludes items that went through rework
- [ ] CLI dashboard renders live factory state with WIP counts per station
- [ ] Metrics persist across restarts (SQLite-backed)
- [ ] Back-pressure events are recorded and visible in the dashboard
- [ ] `synodic trends` shows improvement when Nemosis reduces model tier for a station

## Notes

The metrics store intentionally uses an immutable event log (append-only) with pre-computed aggregates for query performance. Raw events are retained for escape analysis (spec 040) and continuous improvement (spec 042).

Agent utilization is measured at the station level, not the individual agent level, to avoid the complexity of tracking individual agent assignment in this phase.