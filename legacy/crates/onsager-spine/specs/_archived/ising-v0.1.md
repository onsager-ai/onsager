# Ising — Onsager v0.1

**Status**: Draft
**Owner**: Marvin
**Last updated**: 2026-04-13
**Depends on**: `artifact-model-v0.1.md`, `subsystem-map-v0.1.md`
**Related**: `forge-v0.1.md` (defines the Ising feedback protocol from Forge's side in section 7)

---

## 1. Purpose

Ising is the **continuous improvement engine** of Onsager. It observes the entire factory and surfaces insights that make the factory smarter over time. Which shaping patterns fail repeatedly? Which artifact kinds take too long? Which prompts succeed? Which sessions waste tokens? Ising watches, analyzes, and reports.

Ising does not drive production. It does not gate actions. It does not run agents. It **observes** — and its observations flow through well-defined channels to the subsystems that do act. The value of Ising is not in its outputs alone, but in the closed loop they enable: observation feeds insight, insight feeds rule proposals, approved rules gate future actions, and the factory improves.

The name honors Ernst Ising — but within Onsager, "Ising" is a concept, not a person. The Ising model describes how local interactions produce global patterns; Ising the subsystem does the same for factory behavior. Individual events on the spine are local; the patterns Ising detects are global.

---

## 2. Responsibilities

Ising does four things and only four things:

1. **Observe** — continuously read the factory event spine and maintain an internal model of factory behavior over time
2. **Analyze** — run heuristic analyzers over the accumulated observations to detect patterns worth surfacing
3. **Report** — emit insights to the factory event spine and forward them to Forge's scheduling kernel via the advisory protocol
4. **Propose** — when an insight has sufficient confidence and recurrence, package it as a rule proposal and submit it to Synodic's crystallization pipeline

Everything else is **out of scope**. Specifically:

- Ising does **not** drive artifact state. Forge does.
- Ising does **not** run agent sessions. Stiglab does.
- Ising does **not** evaluate governance. Synodic does.
- Ising does **not** store artifact content. External systems do.
- Ising does **not** block production. By design.

Ising is a **read-heavy, write-light** subsystem. Its primary input is the entire factory event spine. Its primary outputs are insight records (written back to the spine) and advisory messages (forwarded to Forge and Synodic).

---

## 3. Mental model

```
         ┌──────────────────────────────────────────────┐
         │                   ISING                      │
         │                                              │
         │   ┌────────────┐      ┌──────────────┐      │
         │   │  Spine     │ ───► │  Analyzer    │      │
         │   │  reader    │      │   registry   │      │
         │   └────────────┘      └──────────────┘      │
         │         ▲                     │              │
         │         │              patterns detected     │
         │    factory events             │              │
         │         │                     ▼              │
         │   ┌────────────┐      ┌──────────────┐      │
         │   │  Spine     │ ◄─── │   Insight    │      │
         │   │  writer    │      │   emitter    │      │
         │   └────────────┘      └──────────────┘      │
         │                              │              │
         │                       ┌──────┴───────┐      │
         │                       ▼              ▼      │
         │               ┌──────────┐   ┌──────────┐   │
         │               │  Forge   │   │ Synodic  │   │
         │               │ advisory │   │ proposal │   │
         │               └──────────┘   └──────────┘   │
         └──────────────────────────────────────────────┘
```

Ising's core loop is:

```
loop:
    events = spine.read_continuous()
    for event in events:
        model.ingest(event)
    for analyzer in analyzer_registry:
        insights = analyzer.run(model)
        for insight in insights:
            spine.write(insight)
            if insight.scope.relevant_to_scheduling():
                forge.forward_advisory(insight)
            if insight.confidence >= crystallization_threshold:
                synodic.propose_rule(insight)
```

This loop is deliberately simple. Complexity lives in the individual analyzers — which are pluggable — not in the loop itself.

---

## 4. Primary input: the factory event spine

Ising is the only subsystem whose **main input** is the factory event spine rather than direct protocol calls (subsystem-map section 4.2). This is because Ising's job is to observe the factory as a whole, not to respond to specific requests.

### 4.1 What Ising reads

Ising reads **every event type** on the factory event spine. The authoritative list of Forge events is in `forge-v0.1 section 9`. Ising also reads events written by Stiglab (session lifecycle upgrades) and Synodic (rule changes, escalation outcomes).

Ising does **not** read session-internal events. It operates at the factory event layer only. This is the abstraction discipline defined in `onsager-vision-v0.1 section 6` — session-internal events stay inside Stiglab; factory events rise to the spine.

### 4.2 Event accumulation

Ising maintains an internal temporal model of factory behavior. This is not a raw event log — it is a structured representation that tracks:

- Artifact state transition histories
- Shaping request/result pairs with timing
- Gate verdict patterns per artifact kind and transition type
- Session duration and outcome distributions
- Insight history (its own previous outputs)

The model is append-oriented. Old data may be compacted or summarized after a configurable retention window, but within the window, full event-level granularity is preserved.

### 4.3 Spine consumption semantics

Ising is a **continuous consumer** of the spine. It does not poll; it subscribes via `pg_notify` and processes events as they arrive. If Ising falls behind (restart, slow analyzer), it catches up from its last-processed position in the outbox table. Catch-up must be idempotent — reprocessing the same event must not produce duplicate insights.

---

## 5. Insight types

Insights are Ising's primary output. Each insight is a structured observation about factory behavior, backed by evidence from the event spine.

### 5.1 Contract

This is the same contract defined in `forge-v0.1 section 7.2`, reproduced here as the authoritative definition from Ising's side:

```
Insight:
    insight_id: InsightId
    kind: Failure | Waste | Win | Anomaly
    scope: ArtifactKind | SpecificArtifact | Global
    observation: string
    evidence: [FactoryEventRef]
    suggested_action: Option<SuggestedAction>
    confidence: float
```

### 5.2 Insight kinds

| Kind | What it means | Example |
|---|---|---|
| **Failure** | A pattern of repeated failures that suggests a systemic issue, not bad luck | "Shaping requests for `code` artifacts with `shaping_intent.language = Python` have failed 7 of last 10 times" |
| **Waste** | Resources (tokens, time, sessions) being spent without proportional value | "Artifact `art_8a3f` has been shaped 12 times without advancing past `in_progress`" |
| **Win** | A pattern that works well and should be reinforced or replicated | "Shaping intents that include explicit test criteria produce `released` artifacts 3x faster" |
| **Anomaly** | Something statistically unusual that does not fit the other categories | "Gate escalation rate for `report` artifacts jumped from 5% to 40% this week" |

### 5.3 Scope

Each insight has a scope that determines who should care about it:

- **ArtifactKind** — the pattern applies to all artifacts of a given kind (e.g., all `code` artifacts)
- **SpecificArtifact** — the pattern applies to one artifact (e.g., `art_8a3f` specifically)
- **Global** — the pattern applies factory-wide, not tied to any artifact or kind

### 5.4 Evidence requirement

Every insight **must** cite at least one factory event as evidence. An insight with empty `evidence` is a bug. This is not a soft guideline — it is a structural invariant. The evidence chain from insight back to factory events is what makes Ising's observations auditable and disputable.

### 5.5 Confidence

`confidence` is a float in `[0.0, 1.0]`. It represents the analyzer's self-assessed certainty that the pattern is real and not noise. Confidence is not a quality score of the suggested action — it is a signal-to-noise measure of the observation itself.

Confidence thresholds govern downstream behavior:

- Below `0.3` — insight is recorded on the spine but not forwarded to Forge
- `0.3` to `0.7` — insight is forwarded to Forge as advisory
- Above `0.7` — insight is forwarded to Forge **and** eligible for rule proposal to Synodic

These thresholds are configurable. The defaults above are v0.1 starting points.

---

## 6. Analysis patterns

Ising v0.1 ships with heuristic analyzers, not learned models. Each analyzer looks for a specific pattern in the accumulated factory model.

### 6.1 Repeated shaping failures

**Looks for**: artifacts or artifact kinds where shaping requests consistently return `Failed` or `Aborted` outcomes.

**Signal**: if more than N of the last M shaping requests for a given scope have failed, emit a `Failure` insight. Default: N=3, M=5.

**Suggested action**: deprioritize the failing scope; alert the operator; review the `shaping_intent` template for the affected kind.

### 6.2 Stuck artifacts

**Looks for**: artifacts that remain in `in_progress` or `under_review` for longer than a configurable duration, or that have been shaped many times without advancing state.

**Signal**: if an artifact has been in a non-terminal state for longer than `kind.expected_duration * 3`, or has accumulated more than `kind.expected_shapings * 2` shaping attempts, emit a `Waste` insight.

**Suggested action**: escalate to operator; review whether the artifact's `shaping_intent` is achievable.

### 6.3 Token waste patterns

**Looks for**: sessions that consume disproportionate resources (duration, inferred token usage from `ShapingResult.duration_ms`) relative to the value they produce.

**Signal**: if a shaping result's duration is more than 3 standard deviations above the mean for its artifact kind, and the outcome is `Partial` or `Failed`, emit a `Waste` insight.

**Suggested action**: cap session duration for the affected scope; review `shaping_intent` complexity.

### 6.4 Successful prompt patterns

**Looks for**: `shaping_intent` configurations that consistently produce `Completed` outcomes with high quality signals.

**Signal**: if a particular `shaping_intent` structure (normalized) has produced `Completed` with above-median quality signals in more than N of the last M attempts, emit a `Win` insight.

**Suggested action**: promote this pattern; suggest it as a template for similar artifact kinds.

### 6.5 Quality signal trends

**Looks for**: directional trends in quality signals across versions of the same artifact, or across artifacts of the same kind.

**Signal**: if quality signals for a given dimension are monotonically declining over the last N versions, emit a `Failure` insight. If monotonically improving, emit a `Win` insight.

**Suggested action**: for declining trends, flag for review; for improving trends, identify what changed and reinforce it.

### 6.6 Gate verdict patterns

**Looks for**: anomalous changes in Synodic gate behavior — sudden spikes in denials, escalations, or modifications for a given artifact kind or transition type.

**Signal**: if the denial or escalation rate for a scope deviates by more than 2 standard deviations from its rolling average, emit an `Anomaly` insight.

**Suggested action**: surface to operator; may indicate a new Synodic rule that is too aggressive or a new class of problematic shaping output.

### 6.7 Scheduling inefficiencies

**Looks for**: patterns where the scheduling kernel's decisions lead to suboptimal outcomes — e.g., consistently dispatching work to overloaded Stiglab nodes, or scheduling low-priority work when high-priority artifacts are blocked.

**Signal**: if artifacts with higher declared priority are consistently shaped after lower-priority ones, or if dispatched work is frequently `Aborted` due to resource contention, emit a `Waste` insight.

**Suggested action**: suggest kernel parameter adjustments; surface to operator.

### 6.8 Analyzer pluggability

The analyzer registry is a **replaceable module**, analogous to Forge's scheduling kernel. v0.1 ships with the heuristic analyzers listed above. Future versions may add learned analyzers, domain-specific analyzers, or user-authored analyzers. The contract is:

```
trait Analyzer:
    fn name(&self) -> &str
    fn run(&self, model: &FactoryModel) -> Vec<Insight>
```

Any implementation that honors this contract and produces well-formed insights (with evidence, valid confidence, correct scope) is valid.

---

## 7. Insight lifecycle

An insight progresses through a defined lifecycle:

```
detected → validated → forwarded → [crystallized]
```

### 7.1 Detected

An analyzer identifies a pattern and produces a candidate insight. At this stage, the insight exists only inside Ising's process.

### 7.2 Validated

Ising checks the candidate insight against deduplication and suppression rules:

- **Deduplication**: if an equivalent insight (same kind, same scope, overlapping evidence) was emitted within a configurable window, the new one is suppressed or merged. Ising does not flood the spine with the same observation.
- **Minimum confidence**: insights below the recording threshold (`0.3` by default) are dropped.
- **Evidence check**: insights with empty evidence are rejected as malformed.

If the insight passes validation, it is written to the factory event spine as an `ising.insight_detected` event.

### 7.3 Forwarded

Based on confidence thresholds (section 5.5), validated insights are forwarded:

- To **Forge** via the advisory protocol defined in `forge-v0.1 section 7`. Forge's scheduling kernel `observe`s the insight as part of its world state. How Forge uses it is kernel-specific and not Ising's concern.
- Forge emits `forge.insight_observed` when it receives the forwarded insight.

### 7.4 Crystallized (optional)

Insights that meet the crystallization threshold are packaged as **rule proposals** and submitted to Synodic's rule crystallization pipeline:

- The proposal includes the insight's observation, evidence, and a suggested rule formulation.
- Synodic evaluates the proposal through its own review process (human or automated).
- If approved, the rule becomes a static governance rule that gates future Forge actions.
- If rejected, the rejection reason is fed back to Ising as a learning signal (v0.1: logged only; future versions may use it to tune analyzer thresholds).

This is the full loop of continuous improvement: **observation (Ising) feeds rule proposal (Ising to Synodic) feeds governance (Synodic to Forge) feeds better production (Forge to Stiglab) feeds new observations (spine to Ising)**.

The crystallization path is how Ising's advisory relationship gains enforcement power without violating the advisory-only invariant. Ising never tells Forge what to do. Ising tells Synodic what it has observed, and Synodic — through its own process — may choose to make it a rule.

---

## 8. Protocols

### 8.1 Ising to Forge: advisory

As defined in `subsystem-map-v0.1 section 3.3` and `forge-v0.1 section 7`.

**Mode**: Advisory. Forge may or may not act on insights. Ising has no mechanism to compel.

**Delivery**: Ising forwards insights above the advisory confidence threshold to Forge. Forge's scheduling kernel `observe`s them. The kernel's response is kernel-specific — a baseline kernel may deprioritize artifacts matching a known failure pattern; a learning kernel may use insights as training signal; a human-in-the-loop kernel may surface insights to operators.

**Non-negotiable**: Ising cannot block Forge. This is structural, not a policy choice. There is no "emergency imperative channel" from Ising to Forge. Adding one would collapse the advisory/gated distinction and weaken governance (subsystem-map section 5.5).

### 8.2 Ising to Synodic: rule proposal

**Mode**: This is not one of the three direct protocols in `subsystem-map-v0.1 section 4.1`. Rule proposals flow through the factory event spine as `ising.rule_proposed` events, which Synodic consumes.

**Contract**:

```
RuleProposal (Ising → Synodic, via spine):
    proposal_id: ProposalId
    source_insight_id: InsightId
    observation: string
    evidence: [FactoryEventRef]
    suggested_rule: RuleFormulation
    confidence: float
```

`RuleFormulation` describes the proposed governance rule in structured form — what gate point it applies to, what condition triggers it, and what verdict it suggests. The exact schema is defined by the Synodic spec; Ising only populates it.

**Synodic's autonomy**: Synodic is under no obligation to accept, or even acknowledge, a rule proposal. Ising does not follow up, does not retry, does not escalate. If a proposal is rejected, Ising records the rejection and may adjust analyzer confidence accordingly in future versions.

### 8.3 Ising reads the factory event spine

This is Ising's primary input channel, not a protocol between two subsystems. Ising subscribes to the spine via `pg_notify` and reads all event types continuously. See section 4 for details.

### 8.4 Ising writes insight records to the spine

Ising writes the following factory events:

| Event | When |
|---|---|
| `ising.insight_detected` | New insight passes validation and is recorded |
| `ising.insight_suppressed` | Insight was deduplicated or below confidence threshold (low frequency, for audit) |
| `ising.rule_proposed` | Insight packaged as a rule proposal for Synodic |
| `ising.analyzer_error` | An analyzer encountered an error during its run |
| `ising.catchup_completed` | Ising finished catching up from a lag position |

This list is **authoritative**. Ising implementations must emit exactly these event types for these situations. Adding new event types is a versioned change to this spec.

---

## 9. Invariants

Ising must maintain these invariants. Violation is a bug:

1. **Advisory-only** — Ising has no imperative or gated path to any subsystem. It cannot block production, cannot deny gate requests, cannot force scheduling decisions. Its only paths to influencing the factory are advisory forwarding to Forge and rule proposals to Synodic.

2. **Evidence-backed** — Every insight must cite at least one factory event in its `evidence` field. Insights without evidence are malformed and must be rejected before they reach the spine.

3. **Non-blocking** — Ising failure does not stop the factory. If Ising crashes, restarts, or falls behind, Forge continues to schedule, Stiglab continues to shape, Synodic continues to gate. The factory runs without Ising; it just does not improve.

4. **Idempotent observation** — Reprocessing the same factory event must not produce duplicate insights. Ising's deduplication logic must handle replays, catch-up, and at-least-once delivery from the spine.

5. **No hidden state mutation** — Ising does not write to any subsystem's storage. It writes only to the factory event spine (its own events) and sends advisory messages through defined protocols. It does not touch artifact state, session state, or rule state.

6. **Spine-only input** — Ising does not read another subsystem's internal database, filesystem, or in-memory state (subsystem-map section 5.2). Its sole data source is the factory event spine.

7. **Insight immutability** — Once an insight is written to the spine, it cannot be modified or retracted. If an insight is later determined to be wrong, a new corrective insight is emitted — the original stands as historical record.

8. **Bounded noise** — Ising must not flood the spine. Deduplication windows, minimum confidence thresholds, and rate limiting ensure that the insight stream remains consumable. The ratio of insight events to factory events is a health signal — if Ising is emitting more insights than there are factory events, something is wrong.

---

## 10. Ising's own state machine

Ising as a running process has three states:

```
running ──pause──► paused
   ▲                  │
   └────── resume ────┘
   │
   ▼
draining ──drained──► stopped
```

- **running** — normal operation, consuming spine events and producing insights
- **paused** — spine reader continues to consume events (to avoid falling behind), but analyzers are not run and no insights are emitted. Useful for maintenance or when insight noise needs to be temporarily silenced.
- **draining** — spine reader stops, in-flight analyzer runs complete, then Ising stops
- **stopped** — fully halted, no activity

Pausing Ising has **zero effect on the rest of the factory**. This is not true of pausing Forge (which stops scheduling) or pausing Synodic (which would stop governance). Ising's pause is the safest pause in the system.

---

## 11. What Ising explicitly is not

To prevent scope creep, these are non-responsibilities:

- **Not a monitoring dashboard** — Ising produces structured insights, not charts or dashboards. Downstream systems (the Console, external observability tools) may consume Ising's outputs for display, but Ising does not render anything.

- **Not a second governance layer** — Ising observes and advises. It does not gate, deny, or escalate. The moment Ising gains the power to block production, it becomes a governance subsystem and the clean separation between observation and enforcement collapses. If something Ising observes needs to become a rule, it goes through Synodic.

- **Not a learned model (v0.1)** — v0.1 uses heuristic analyzers with configurable thresholds. There are no neural networks, no reinforcement learning, no statistical models that train on the factory's data. This is a deliberate constraint — heuristics are inspectable, debuggable, and predictable. Learned models may be introduced as analyzer plugins in future versions, once the insight pipeline is battle-tested.

- **Not a metrics aggregator** — Ising does not compute averages, percentiles, or time-series metrics for general consumption. It detects **actionable patterns**. The difference: "average shaping duration is 45 seconds" is a metric; "shaping duration for `code` artifacts has tripled this week and 60% of those shapings fail" is an insight. Ising produces the latter.

- **Not a real-time alerting system** — Ising operates on accumulated factory behavior, not individual events. It does not page operators when a single shaping fails. It surfaces patterns. The latency from event to insight is measured in minutes, not milliseconds.

- **Not an artifact quality scorer** — Artifact quality signals are appended by Forge based on Stiglab's shaping results and external evaluators (artifact-model section 4.6). Ising reads quality signals as part of its input, but it does not produce them. Ising produces insights *about* quality trends, not quality scores themselves.

---

## 12. Relationship to the Ising sibling repo

The `ising` repository under `onsager-ai/` is a **code maintainability analysis engine** — a standalone tool that builds multi-layer graphs of codebases and computes risk scores, safety factors, and signals. It predates Onsager's subsystem architecture.

The relationship between the Ising subsystem (this spec) and the `ising` repo (the code analysis engine) is:

- The `ising` repo's analysis capabilities are a **potential analyzer plugin** for the Ising subsystem. Specifically, for `code` artifacts, the code graph analysis could provide quality signals and structural insights that feed into Ising's factory-level patterns.
- The Ising subsystem is **not** the `ising` repo. The subsystem observes the entire factory event spine across all artifact kinds. The repo analyzes code structure specifically.
- Integration path: in a future version, the `ising` repo may be invoked as part of Stiglab's shaping process (to assess code artifacts) or as an Ising analyzer plugin (to enrich `code`-scoped insights with structural analysis). This integration is out of scope for v0.1.

---

## 13. Open questions

1. **Analyzer execution model** — Do analyzers run on every event, on a fixed tick, or on a combination? Per-event analysis is responsive but expensive for complex analyzers. Tick-based analysis is efficient but introduces latency. v0.1 leans toward tick-based with configurable intervals per analyzer.

2. **State persistence** — Ising's internal factory model must survive restarts. What is the storage backend? Options: PostgreSQL (same as the spine, separate schema), SQLite (lightweight, local), or in-memory with catch-up from spine on restart. Catch-up from spine is simplest but slow for long histories. Leaning toward PostgreSQL with a dedicated schema.

3. **Backpressure** — If Ising falls far behind the spine (extended outage, slow analyzers), how does it catch up without overwhelming downstream consumers with a burst of stale insights? Should there be a staleness threshold beyond which old observations are discarded rather than emitted?

4. **Cross-insight correlation** — Can Ising combine multiple lower-confidence insights into a higher-confidence composite insight? v0.1 does not support this. Each analyzer runs independently. Correlation across analyzers is a v0.2 concern.

5. **Insight expiry** — Do insights have a TTL? A `Waste` insight about a stuck artifact becomes irrelevant if the artifact is subsequently released. Should Ising emit expiry events, or should consumers handle staleness themselves?

6. **Crystallization threshold tuning** — The confidence threshold for rule proposals (section 5.5) is global. Should it be per-analyzer, per-artifact-kind, or per-insight-kind? Different pattern types may warrant different crystallization sensitivities.

7. **Feedback from Synodic** — When Synodic rejects a rule proposal, should the rejection reason be structured enough for Ising to adjust its behavior automatically, or is logging sufficient for v0.1?

8. **Multi-instance Ising** — Can multiple Ising instances run in parallel for availability? If so, how do they coordinate to avoid duplicate insights? Deduplication on the consumer side (Forge, Synodic) is one option; leader election among Ising instances is another. Not addressed in v0.1.

---

## 14. Anti-patterns

The following practices are antipatterns in Ising and should be blocked by tooling or review:

1. **Evidenceless insight** — an insight without factory event references. Violates invariant 2.

2. **Imperative insight** — an insight that triggers an automatic action in Forge without the scheduling kernel's intermediation. Ising outputs are advisory. Always.

3. **Insight flood** — emitting insights at a rate comparable to or exceeding the factory event rate. Violates invariant 8. If this happens, deduplication windows are too narrow or confidence thresholds are too low.

4. **Direct database read** — Ising querying Forge's artifact table, Stiglab's session store, or Synodic's rule database. Violates invariant 6 and subsystem-map section 5.2. All data comes through the spine.

5. **Quality signal injection** — Ising writing quality signals to artifacts. Quality signals are appended by Forge based on shaping results and external evaluators (artifact-model section 4.6). Ising reads them; it does not produce them.

6. **Silent suppression** — dropping an insight without emitting `ising.insight_suppressed`. Suppressions are normal (deduplication is healthy), but they must be auditable.

7. **Analyzer side effects** — an analyzer that modifies the factory model, writes to external systems, or communicates with other subsystems. Analyzers are pure functions of the model. Side effects belong in the insight emitter.

8. **Governance masquerading** — framing an insight's `suggested_action` so specifically that it becomes a de facto gate verdict. Suggested actions are suggestions. They must leave room for the scheduling kernel (Forge) or the rule review process (Synodic) to disagree.

---

## 15. Decision log

| Date | Decision | Rationale |
|---|---|---|
| 2026-04-13 | Ising is advisory-only, no imperative path to any subsystem | Observation and governance are separate concerns; mixing them creates a second governance layer |
| 2026-04-13 | Primary input is the factory event spine, not direct protocol calls | Ising observes the factory as a whole; protocol calls would couple it to specific subsystems |
| 2026-04-13 | v0.1 uses heuristic analyzers, not learned models | Heuristics are inspectable, debuggable, predictable; ML analyzers are a future plugin |
| 2026-04-13 | Insights must cite factory events as evidence | Auditability and disputability require a traceable evidence chain |
| 2026-04-13 | Rule proposals go through Synodic's crystallization pipeline | This is how advisory gains enforcement power without violating the advisory invariant |
| 2026-04-13 | Analyzer registry is pluggable | Avoid locking analysis approach at spec level; same principle as Forge's kernel pluggability |
| 2026-04-13 | Ising failure does not stop the factory | The factory runs without improvement; it does not run without production or governance |
| 2026-04-13 | Insight immutability — no retraction, only correction | Same principle as quality signal append-only in the artifact model |
| 2026-04-13 | Confidence thresholds govern downstream routing | Separates recording (all validated insights) from acting (forwarded/proposed insights) |
| 2026-04-13 | Ising writes its own event types to the spine | Makes Ising's contract with the rest of Onsager explicit, same as Forge's event list |
