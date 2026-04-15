---
status: archived
created: 2026-03-06
priority: critical
tags:
- core
- fleet
- orchestration
- message-bus
- master-worker
- ai-native
- coordination
- cost-optimization
depends_on:
- clawden:012-fleet-orchestration
created_at: 2026-03-06T06:56:22.809363808Z
updated_at: 2026-03-06T06:56:22.809456972Z
---
# Agent Fleet Execution Layer — Master-Worker Orchestration, Message Bus & Task Lifecycle

## Overview

This spec is the umbrella for the fleet execution layer — running agents, passing messages between them, collecting results, persisting state, and coordinating sophisticated multi-agent workflows.

The work splits into five layers that build on each other:

1. **Execution substrate** — agents running and staying alive.
2. **Collaboration protocol** — agents communicating and working together on tasks.
3. **Reliability layer** — fleet state surviving crashes and restarts.
4. **Coordination intelligence** — advanced and AI-native coordination patterns that go beyond simple master-worker.
5. **Cost optimization** — teacher-student knowledge distillation to reduce fleet operating costs.

Layers 1–3 are single-host by design. Distributed execution (cross-host message relay, remote supervisor) builds on top of spec 009's control channel, reusing the same `AgentEnvelope` protocol and supervisor interface. Layers 4–5 are transport-agnostic and work identically on local and distributed fleets.

## Design

This umbrella coordinates two groups spanning the five layers:

### Group D: Execution Foundation — Layers 1–3 (003)

The strictly sequential critical path for single-host fleet execution:

| Child                                      | Layer | Purpose                                                                                                              |
| ------------------------------------------ | ----- | -------------------------------------------------------------------------------------------------------------------- |
| `004-fleet-process-supervisor`             | 1     | Spawn agents, attach pipes, health probes, supervised restart, graceful shutdown, fleet config parsing, `clawden up` |
| `005-agent-message-bus-task-orchestration` | 2     | In-process message bus, `AgentEnvelope` protocol, team coordination, task lifecycle engine, result aggregation       |
| `006-fleet-state-persistence-recovery`     | 3     | SQLite backend for agents/teams/tasks/results/messages/audit, crash recovery, `clawden logs`/`clawden audit`         |

### Group E: Coordination & Optimization — Layers 4–5 (011)

ClawDen's implementation binding of the abstract coordination model (spec 017), plus cost optimization via Nemosis:

| Child                                      | Layer | Purpose                                                                                                                                                                                                           |
| ------------------------------------------ | ----- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `012-advanced-coordination-patterns`       | 4     | Pluggable org-chart patterns: hierarchy, pipeline, committee, marketplace — mapping real-world organizational structures onto agent fleets                                                                        |
| `013-ai-native-coordination-primitives`    | 4     | Primitives with no human analogue: speculative swarm, context mesh, fractal decomposition, generative-adversarial, stigmergic — exploiting zero fork cost, lossless context transfer, and speculative parallelism |
| `014-ai-native-domain-playbooks`           | 4     | Applied compositions of AI-native primitives for concrete domains: software engineering, finance, marketing, research, legal, devops                                                                              |
| `015-sdd-ai-native-playbook`              | 4     | AI-native playbooks applied to spec-driven development itself — spec exploration, hardening, living graph maintenance, fractal decomposition                                                                      |
| `016-nemosis-teacher-student-distillation` | 5     | Nemosis integration for teacher-student knowledge distillation — captures fleet execution traces, distills them into SKILL.md artifacts, routes subsequent runs to cheaper student models with iterative memory-backed refinement |

Shared architectural rules:

- JSON-Lines over stdin/stdout is the agent communication wire format.
- `AgentEnvelope` is the stable message protocol used by both local and (future) remote delivery.
- The process supervisor owns agent lifecycle; the message bus owns routing; persistence is the durability layer underneath both.
- Master-worker is the foundational collaboration pattern; advanced and AI-native patterns (012–015) are pluggable coordination strategies on top of the same bus.
- AI-native primitives (013) extend the coordination trait surface with `spawn`, `merge`, `fork`, `observe`, `convergence`, and `prune` operations that exploit properties unique to AI agents.
- Nemosis (016) operates as a sidecar that captures traces, distills skills, and informs the scheduler's model selection — reducing fleet cost by 50–90% for repetitive patterns.

## Plan

- [ ] Complete spec 004 to establish agent process management and fleet startup.
- [ ] Complete spec 005 to add inter-agent messaging and task orchestration on top of the running fleet.
- [ ] Complete spec 006 to make fleet state persistent and recoverable.
- [ ] Complete spec 012 to add pluggable organizational coordination patterns.
- [ ] Complete spec 013 to define AI-native coordination primitives.
- [ ] Complete specs 014–015 to map primitives to domain and SDD playbooks.
- [ ] Complete spec 016 to integrate Nemosis for fleet cost optimization via distillation.

## Test

- [ ] A fleet of 3+ heterogeneous agents starts, stays healthy, and shuts down cleanly.
- [ ] A master-worker task flow produces aggregated results from multiple workers.
- [ ] Fleet state survives a crash and resumes on restart.
- [ ] Advanced coordination patterns (hierarchy, pipeline, committee) produce correct results via the same message bus.
- [ ] AI-native primitives (speculative swarm, context mesh) produce outputs no single agent could achieve alone.
- [ ] Nemosis distillation reduces fleet cost by routing repetitive agent roles to student models without quality degradation.

## Notes

Implementation order for the foundation is strictly sequential: 004 → 005 → 006. Each layer depends on the previous one.

The coordination intelligence layer (012–015) builds on the foundation but is internally layered: 012 (org-chart patterns) → 013 (AI-native primitives) → 014 (domain playbooks) → 015 (SDD playbook). Each extends the previous.

The cost optimization layer (016) depends on the coordination primitives (013–014) and hooks into all three foundation layers: trace capture via the process supervisor (004), message observation via the bus (005), and trace persistence via the SQLite backend (006).

### Relationship to Spec 017

**Spec 017 (AI-Native Agent Coordination Model)** defines the abstract, implementation-agnostic coordination model that Layers 4–5 implement. It lives outside this umbrella because the model is portable — other frameworks can implement the same primitives, operations, and playbook compositions independently.

This umbrella (002) and its children (012–016) are **ClawDen's implementation binding** of spec 017's abstract model: Rust traits, AgentEnvelope wire format, SQLite persistence, `clawden.yaml` config, and CLI commands. The abstract model (017) is the *what*; this umbrella is the *how*.

The distributed story connects here:
- Spec 009 (remote enrollment + control channel) provides the transport for cross-host message relay.
- Spec 005's `AgentEnvelope` format is the protocol that travels over that transport.
- A future spec can add a `RemoteMessageBus` backend that routes envelopes through 009's control channel, swapping the tokio channel backend without changing the bus API.