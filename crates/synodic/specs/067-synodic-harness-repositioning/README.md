---
status: in-progress
created: 2026-03-24
priority: critical
tags:
- vision
- harness
- governance
- architecture
- repositioning
created_at: 2026-03-24T06:00:39.814860715Z
updated_at: 2026-03-24T06:00:39.814860715Z
---

# Synodic Repositioning — Open-Source AI Agent Governance Platform

## Overview

Reposition Synodic from an "AI coding factory" (BUILD / INSPECT pipelines, coordination patterns) to a **self-contained, self-hostable AI agent event governance platform**. Synodic becomes a focused product: monitor, audit, and enforce governance rules on AI coding agent sessions.

Coordination patterns (factory, fractal, swarm, adversarial) and the eval framework move to separate repositories. What remains is a platform that answers: **"What did the AI agent do, and was it correct?"**

## Vision

Synodic is an open-source AI agent governance platform. It collects events from AI coding tool sessions (Claude Code, GitHub Copilot, Cursor, etc.), classifies them using detection rules, surfaces issues through a dashboard and CLI, and crystallizes recurring patterns into enforceable rules.

**Core identity:** The tool that watches the AI agents.

**Product surface:**
- **CLI** — submit events, collect from session logs, query, resolve, watch live
- **Web Dashboard** — event feed, resolution queue, analytics, rules manager
- **TUI** — terminal-based live event monitoring
- **Skill** — harness-governance skill makes agents self-reporting
- **REST API** — programmatic access for integrations

**Distribution model (follows LeanSpec):**
- npm-published CLI with platform-specific Rust binaries
- Docker image for self-hosting
- Skill installable via `npx skills add`
- Deploy configs for Fly.io, Railway, Render

## Architecture

### Repository structure

```
synodic/
├── rust/
│   ├── Cargo.toml                    # workspace: harness-core, harness-cli, harness-http
│   ├── harness-core/                 # Event types, detection rules, storage, parsers
│   ├── harness-cli/                  # CLI: submit, collect, query, resolve, watch, serve
│   └── harness-http/                 # Axum REST API + serves dashboard static files
├── packages/
│   ├── cli/                          # npm wrapper for Rust binary
│   └── ui/                           # Vite React dashboard (Tailwind, Radix, TanStack Query)
├── skills/
│   └── harness-governance/           # SKILL.md + agent instructions
│       └── SKILL.md
├── docs-site/                        # Docusaurus documentation
├── docker/                           # Multi-stage Dockerfile
├── deploy/                           # Fly.io, Railway, Render configs
├── schemas/                          # Event JSON schemas
├── specs/                            # LeanSpec specs (harness-scoped only)
├── HARNESS.md                        # Governance protocol
└── package.json                      # pnpm monorepo
```

### Rust crate mapping (LeanSpec-aligned)

| Crate | Role | LeanSpec equivalent |
|-------|------|---------------------|
| `harness-core` | Event types, detection rules, storage abstraction, log parsers | `leanspec-core` |
| `harness-cli` | CLI commands: submit, collect, query, resolve, rules, watch, serve | `leanspec-cli` |
| `harness-http` | Axum REST API, WebSocket for live events, serves dashboard | `leanspec-http` |

### Storage

- **SQLite** — default for local development and single-developer use. Zero-config, works out of the box.
- **PostgreSQL** — optional for team/org deployments. Same schema, storage abstraction switches based on `DATABASE_URL`.
- Storage abstraction trait in `harness-core` — implementations for both backends.

### CLI commands

```bash
# Setup
synodic init                              # initialize .harness/ in project
synodic auth login                        # configure API key (for remote mode)
synodic serve                             # start API + dashboard (localhost:3000)

# Event submission
synodic submit --type <type> --title "<title>" [--severity <level>] [--metadata '<json>']

# Log collection (passive)
synodic collect [--source claude|copilot|auto] [--since <duration>] [--dry-run]

# Query
synodic list [--type <type>] [--severity <level>] [--unresolved]
synodic search "<query>"
synodic stats [--since <duration>]

# Resolution
synodic resolve <id> [--notes "<notes>"]

# Rules
synodic rules list
synodic rules test <rule> --against <log>
synodic rules add <pattern>

# Live monitoring
synodic watch [--filter "<expr>"]         # TUI: live event stream
```

## Core capabilities

### Event types
- `tool_call_error` — tool execution failures
- `hallucination` — references to nonexistent files/APIs
- `compliance_violation` — secrets, dangerous commands, prod access
- `misalignment` — agent actions diverge from user intent

### Detection rules engine
- Pattern-based matching (regex, structural)
- Source-specific parsers (Claude Code JSONL, Copilot events.jsonl)
- Configurable severity thresholds
- Rule crystallization: pattern detected 3+ times -> candidate L1 rule -> backtest -> promote

### Two-layer governance (from HARNESS.md)
- **L1**: Static/deterministic rules (zero AI cost, fast)
- **L2**: AI judge (independent LLM, fresh context, semantic analysis)

### harness-governance skill
- SKILL.md teaches agents about event types and governance
- Agent self-reports events it notices (active)
- Agent runs `synodic collect` to scan its own logs (passive)
- Self-audit checklist at end of major tasks

## Scope reduction

### Extract to `codervisor/eval` (standalone)
- Entire `synodic-eval` crate
- `evals/` directory (tasks, setup, results)
- Already has clean separation boundary — zero governance deps

### Extract to `codervisor/orchestra` (coordination patterns)
- Pipeline engine (`pipeline/` — schema, executor, gates, vars, validate, checkpoint)
- Fractal algorithms (`fractal/` — decompose, schedule, reunify, prune)
- Swarm algorithms (`swarm/` — checkpoint, prune)
- Skills: factory, fractal, swarm, adversarial (SKILL.md + prompts + evals)
- Pipeline YAMLs (`.harness/pipelines/`)
- Schemas for pipeline outputs (build-report, inspect-verdict, etc.)

### Spec migration

**Stay in Synodic (harness scope):**

| Spec | Status | Reason |
|------|--------|--------|
| 048-post-session-governance | in-progress | Core governance model |
| 055-harness-real-world-assessment | complete | Harness validation results |
| 056-fix-harness-test-lean-spec | planned | Harness gap fixes |

**Move to `codervisor/orchestra`:**

| Spec | Status | Reason |
|------|--------|--------|
| 037-coding-factory-vision | planned | Factory concept |
| 044-factory-skill-mvp | in-progress | Factory skill |
| 049-factory-test-harness | planned | Factory testing |
| 050-fractal-algorithmic-spine | complete | Fractal algorithms |
| 052-fractal-factory-composition | planned | Coordination composition |
| 058-code-harness-orchestration | complete | Pipeline orchestration |
| 059-context-mesh | draft | Coordination primitive |
| 060-stigmergic-coordination | draft | Coordination primitive |
| 061-pipeline-engine-core | complete | Pipeline engine |
| 062-gate-system | complete | Gates |
| 063-pipeline-definitions | complete | Pipeline YAMLs |
| 064-algorithmic-commands | complete | Fractal/swarm CLI |
| 065-skill-migration | complete | Skill shims |

**Move to `codervisor/eval`:**

| Spec | Status | Reason |
|------|--------|--------|
| 046-synodic-dogfood | complete | Eval dogfood |
| 047-decouple-eval-framework | complete | Eval decoupling |
| 053-test-synodic-harness-lean-spec | complete | Eval test results |
| 066-umbrella-058-test-results | complete | Test assessment |

**Archive (superseded by repositioning):**

| Spec | Status | Reason |
|------|--------|--------|
| 045-rust-consolidation | complete | Old structure, no longer applicable |
| 051-production-roadmap | in-progress | Superseded by this spec |
| 066-ai-meta-testing-framework | in-progress | Moves with eval or gets reworked |

**Already archived (no action):** 001-043 (minus 037), 054, 057

## Migration path

### Phase 1: Extract eval (clean cut)
- [x] Create `codervisor/eval` repo
- [x] Move `cli/synodic-eval/` as the primary crate
- [x] Move `evals/` directory
- [x] Move eval-related specs (046, 047, 053, 066-umbrella)
- [x] Remove synodic-eval from this workspace

### Phase 2: Extract coordination patterns
- [x] Create `codervisor/orchestra` repo
- [x] Move `pipeline/`, `fractal/`, `swarm/` modules
- [x] Move skills (factory, fractal, swarm, adversarial) with prompts and evals
- [x] Move `.harness/pipelines/` and pipeline output schemas
- [x] Move coordination specs (037, 044, 049, 050, 052, 058-065)
- [ ] Orchestra consumes Synodic as a dependency for governance event submission

### Phase 3: Restructure this repo
- [x] `cli/` -> `rust/` with new workspace (harness-core, harness-cli, harness-http)
- [x] Add `packages/` (cli npm wrapper, ui)
- [x] Create `skills/harness-governance/SKILL.md`
- [x] Update CLAUDE.md, HARNESS.md, README.md for new scope
- [x] Archive superseded specs, migrate applicable ones

### Phase 4: Build harness-core
- [x] Event types and schemas
- [x] Storage abstraction trait (SQLite)
- [x] Detection rules engine
- [x] Log parsers (Claude Code)
- [x] Log parsers (Copilot)
- [ ] Log parsers (Cursor)
- [x] PostgreSQL storage backend
- [x] L1 static rule evaluation (refactored from harness/run.rs)

### Phase 5: Build harness-cli
- [x] submit, collect, query, resolve commands
- [x] rules management commands
- [x] watch command (TUI via Ratatui)
- [x] serve command (launches harness-http)
- [x] --since filtering in collect

### Phase 6: Build harness-http + dashboard
- [x] Axum REST API (events CRUD, rules, stats)
- [x] Static file serving for dashboard
- [x] WebSocket for live event streaming
- [x] Vite React dashboard (event feed, resolution queue, analytics)

### Phase 7: Distribution
- [x] npm wrapper with platform-specific binaries
- [x] Docker image (multi-stage build)
- [x] Deploy configs (Fly.io, Railway, Render)
- [x] Docusaurus documentation site
- [x] harness-governance skill

## Non-goals

- **Not a coordination orchestrator** — that is `codervisor/orchestra`
- **Not an eval framework** — that is `codervisor/eval`
- **Not a Nova Harness client** — Synodic is a standalone open-source platform
- **Not SaaS** — self-hostable first, cloud offering is a separate future decision

## Success criteria

- [x] synodic-eval extracted to standalone repo with all tests passing (35 tests, github.com/codervisor/eval)
- [x] Coordination patterns extracted to orchestra repo with all tests passing (140 tests, github.com/codervisor/orchestra)
- [x] This repo builds and serves a working governance dashboard
- [x] `synodic collect --source claude --dry-run` parses real session logs
- [x] `synodic serve` starts API + dashboard on localhost
- [x] harness-governance skill installable and functional
- [x] SQLite works out of the box
- [x] PostgreSQL works with DATABASE_URL (feature-gated via `--features postgres`)