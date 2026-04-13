# Onsager — Vision v0.1

**Status**: Draft
**Owner**: Marvin
**Last updated**: 2026-04-13
**Supersedes**: `onsager-vision-v0.0` (event-stream-centric draft, Jan 2026)
**Related**: `artifact-model-v0.1.md`, `forge-v0.1.md`

---

## 1. The one-line version

> **Onsager is an AI factory that forges artifacts at scale — with governance, observation, and continuous improvement built in.**

Everything that follows is an unpacking of that sentence.

---

## 2. What problem does Onsager solve

Agent systems today are optimized for **running agents well**. Frameworks orchestrate tool calls, runtimes manage context, dashboards show traces. The implicit assumption is that the value is in the running — that a good agent platform is one that runs agents reliably.

This framing has a hole. **Running agents is not the point.** The point is what agents **produce** — the code that ships, the report that informs a decision, the document that gets published, the product that users touch. A platform that runs agents flawlessly but never ties the running to durable, tracked, improvable *output* is solving the wrong problem.

Onsager is built on the opposite assumption: **the value is in the artifacts**. Everything else — orchestration, governance, observation — exists to make artifacts come out faster, better, safer, and more reliably over time.

This is the shift from a **process-oriented** platform to a **deliverable-oriented** factory.

---

## 3. The core concept: artifact

An **artifact** is a persistent, identity-stable, value-bearing thing that Onsager exists to produce. Code, reports, documents, products, configurations, executed API calls — anything that someone will actually use is a candidate.

Artifacts have four properties that distinguish them from the ephemeral byproducts of agent execution:

- **They persist** beyond any single session. A session ends; the artifact continues.
- **They have identity.** An artifact being shaped over 5 sessions is still one artifact, not 5 outputs.
- **They bear value.** Every artifact has a declared consumer. Artifacts with no consumer are an anomaly.
- **They are the factory's purpose.** Rules, insights, policies, events — these exist to make artifacts better, not the other way around.

For the precise data model and invariants, see `artifact-model-v0.1.md`. This vision doc only uses the concept; it does not define it.

---

## 4. The four subsystems

Onsager is not a monolith. It is a family of four subsystems that each do one thing well and cooperate around artifacts.

### Forge — the production line

Forge drives artifacts through their lifecycle. It decides what to shape next, dispatches shaping work, gates state transitions with governance, advances artifact state, and routes finished artifacts to consumers. Forge is the **active core** of Onsager — when Onsager is running, Forge is the thing running.

Forge is the single writer to artifact state. No other subsystem mutates artifacts directly. This single-writer constraint is how Onsager guarantees identity conservation, lineage completeness, and audit integrity.

### Stiglab — the distributed shaping runtime

Stiglab runs the agent sessions that actually shape artifacts. When Forge says "shape artifact X like this," Stiglab allocates a node, spawns agents, manages the session's internal state, and returns a shaping result. Stiglab owns the concept of "session" — Forge only sees shaping requests and shaping results.

The name comes from *stigmergy*, the indirect coordination mechanism social insects use to collaborate without central control. It reflects Stiglab's role as a distributed, node-based runtime where work emerges from local interactions under global constraints.

### Synodic — the governance gate

Synodic evaluates every action that touches an artifact. Before Forge dispatches shaping work, Synodic may deny. Before an artifact advances to a new state, Synodic may deny or escalate. Before a released artifact is routed to consumers, Synodic may redact or block.

Synodic's verdicts are **unconditionally honored** by Forge. There is no override. If governance disagrees with the production line, governance wins — structurally, not by policy. This is how Onsager makes the promise that "governance cannot be bypassed by the factory itself."

Synodic also owns rule crystallization: the process by which repeated decisions (human or AI) become static rules that govern future actions automatically. Rules evolve over time as the factory learns what is safe, useful, and efficient.

The name comes from the astronomical *synodic period* — the cycle of alignment between celestial bodies. Governance in Onsager is a periodic re-alignment between agent action and human intent.

### Ising — the continuous improvement engine

Ising observes the entire factory and surfaces insights that make it smarter over time. Which shaping patterns fail repeatedly? Which artifact kinds take too long? Which prompts succeed? Which sessions waste tokens? Ising watches, analyzes, and reports.

Ising's outputs are **advisory**, not imperative. It cannot block production — its path to influencing future work is through Synodic's rule crystallization pipeline. An insight becomes a proposed rule, a rule is reviewed, an approved rule gates future actions. This is the full loop of continuous improvement.

The name honors Ernst Ising — but within Onsager, "Ising" is a concept, not a person. The Ising model describes how local interactions produce global patterns; Ising the subsystem does the same for factory behavior.

---

## 5. How they fit together

```
                        ┌──────────────┐
                        │   ARTIFACT   │
                        │ (the value)  │
                        └──────▲───────┘
                               │
                    writes     │    references
                               │
                        ┌──────┴───────┐
                        │    FORGE     │
                        │ (production  │
                        │    line)     │
                        └──┬────┬────┬─┘
              dispatches   │    │    │   consults
                           ▼    ▼    ▼
                    ┌──────┐ ┌─────┐ ┌─────┐
                    │STIGLAB│ │SYNO│ │ISING│
                    │(shape)│ │DIC │ │     │
                    │       │ │(gate)│(obsv)│
                    └───────┘ └─────┘ └─────┘
                         ▲       │       │
                         │       │       │
                         └───────┴───────┘
                          factory event
                              spine
                         (pg outbox + notify)
```

Three relationships define the architecture:

**Forge → Stiglab is imperative.** Forge tells Stiglab what to shape; Stiglab executes. One-way command.

**Forge ↔ Synodic is gated.** Forge consults Synodic at every governance point; Synodic's verdict is binding. Synodic cannot be routed around.

**Ising → Forge is advisory.** Ising observes and suggests; Forge may incorporate insights into scheduling but is not compelled. Ising's path to enforcement is through Synodic, not Forge.

These three modes — imperative, gated, advisory — are deliberately different. Collapsing them into one uniform protocol would destroy the subsystem boundaries that make Onsager tractable.

All four subsystems communicate through a shared **factory event spine** — a PostgreSQL outbox table with `pg_notify` delivery. Every state-changing action writes an event to this spine in the same transaction. The event spine is how the subsystems stay consistent without direct coupling.

---

## 6. The two layers of events

A common trap in building agent platforms is mixing two fundamentally different event layers. Onsager keeps them strictly separate.

**Session-internal events** — tool calls, tool results, agent thoughts, subagent spawns. These are high-frequency (dozens per second per session), private to the session, and useful only for debugging. They live inside Stiglab sessions and do not leave them.

**Factory events** — artifact registered, state changed, shaping dispatched, gate verdict received, insight observed. These are low-frequency (a few per minute per cluster), semantically meaningful, and shared across subsystems. They live on the factory event spine.

The crucial move is **upgrading**: when something session-internal has governance or improvement significance, Stiglab upgrades it into a factory event. An ordinary `rm -rf /tmp/cache` stays internal; an `rm -rf /etc/*` upgrades into `policy.escalated`. The ratio of internal events to upgraded factory events is a health signal — if the ratio is too low, the factory is under-governed; if too high, the factory is over-reporting noise.

Synodic and Ising consume the factory event spine. They do **not** see session-internal events. This abstraction discipline is how Onsager scales without its governance layer drowning in noise.

---

## 7. What Onsager is not

To stay sharp, it is worth saying what Onsager is **not**:

- **Not a workflow engine.** Onsager does not ask you to author DAGs of tasks. The "workflow" emerges from horizontal artifact lineage — one artifact referencing another. Users think in terms of artifacts they want, not graphs of steps.
- **Not an agent framework.** Onsager runs on top of agent runtimes (Claude Code, Gemini CLI, Copilot CLI, custom) through Stiglab. It does not reinvent the agent layer.
- **Not a content store.** Onsager holds metadata and pointers. Content lives where it naturally lives — Git, S3, Notion, databases. This keeps Onsager lightweight and respects users' existing systems.
- **Not a monitoring dashboard.** Onsager produces, governs, and improves. Observability is a means, not the end. If Onsager only showed you what agents were doing without driving them to produce better artifacts, it would be a failed product.
- **Not a consumer-facing app.** Onsager is an operator-grade system. Its users are teams building AI-driven production pipelines, not end users of AI assistants.

---

## 8. Who Onsager is for

Onsager is for organizations that want to **continuously produce high-quality artifacts using AI agents, at a scale where manual oversight of each session is impossible.**

Concrete examples:

- A consulting firm that produces industry research reports weekly
- A software team whose PRs are primarily agent-authored
- A content operation generating scripts, articles, or briefs at volume
- An enterprise analytics group producing scheduled test reports (e.g., HP Nova)
- A product team running AI-driven customer research and summarization pipelines

These organizations share three characteristics: **high artifact throughput**, **meaningful quality standards**, and **a need for governance and audit trails**. They cannot accept "the agent did something, trust us." They need a factory, not a toybox.

---

## 9. Why the factory metaphor holds

Factory is not marketing language. It is a precise architectural claim:

| Factory concept | Onsager concept |
|---|---|
| Production line | Forge |
| Workers / machines | Stiglab sessions |
| Quality gates | Synodic |
| Process improvement | Ising |
| Products | Artifacts |
| Bill of materials | Horizontal lineage |
| Production log | Factory event spine |
| Shift supervisor | Operator using the Console |
| Standard operating procedures | Synodic rules |
| Kaizen | Rule crystallization |

The mapping is tight, not loose. Every major factory concept has an exact Onsager counterpart, and every major Onsager component has a factory role. This is why the metaphor survives scrutiny — it is not decoration, it is the actual shape of the system.

The metaphor also gives Onsager its operational language. When an operator sees a stuck artifact, they ask "which workstation is it stuck at?" not "which event handler failed?". When an engineer designs a new subsystem, they ask "what role on the factory floor?" not "what microservice pattern?". The metaphor reduces cognitive load on every decision.

---

## 10. The deeper bet

Onsager is a bet on a specific view of where AI agents are going.

The bet is this: **in the next few years, the most valuable AI agent systems will be the ones that produce durable, governed, improvable artifacts at scale — not the ones that have the smartest models, the slickest chat interfaces, or the cleverest prompt techniques.** Models will commoditize. Prompts will commoditize. What will not commoditize is the operational infrastructure for turning agent capability into trustworthy production output.

Onsager is trying to be that infrastructure.

If the bet is right, the winners in applied AI will not be the teams with the best agents. They will be the teams with the best **factories** around their agents. And every factory needs a production line, a quality gate, a continuous improvement loop, and a clear answer to the question "**what did we produce today?**"

That is what Onsager gives them.

---

## 11. Roadmap stance

Onsager v0.1 is focused on making the four-subsystem architecture real and verifiable — not on feature breadth. The priorities are:

1. Forge's core loop with a baseline scheduling kernel
2. Stiglab alignment with the new shaping-executor role
3. Synodic's gating protocol and rule crystallization pipeline
4. Ising's insight generation on the factory event spine
5. A minimal operator Console that makes the factory visible

Things explicitly **not** in v0.1:

- Multi-tenant architecture
- HA deployment of Forge
- Hot-swappable scheduling kernels
- Non-Claude agent runtimes in Stiglab
- A marketplace of Synodic rules
- Learned scheduling models
- Branching artifact versions
- Mobile-native clients (Console is responsive web only)

These will come — but only after the core architecture has been proven on real workloads.

---

## 12. Decision log

| Date | Decision | Rationale |
|---|---|---|
| 2026-04-13 | Onsager is artifact-centric, not session-centric | The value is in the deliverable, not the running |
| 2026-04-13 | Four subsystems: Forge, Stiglab, Synodic, Ising | Each does one thing well; relationships are asymmetric by design |
| 2026-04-13 | Forge is the single writer to artifact state | Simplifies invariant enforcement |
| 2026-04-13 | Synodic verdicts are structurally unbypassable | Core of the governance story |
| 2026-04-13 | Ising is advisory, routes through Synodic for enforcement | Preserves subsystem cohesion |
| 2026-04-13 | Two event layers (internal / factory) kept strictly separate | Prevents governance layer from drowning in noise |
| 2026-04-13 | Factory metaphor is architectural, not marketing | Every concept has an exact counterpart |
| 2026-04-13 | Onsager does not store content, only metadata and pointers | Respect existing systems; stay lightweight |
