# Synodic — Onsager v0.1

**Status**: Draft
**Owner**: Marvin
**Last updated**: 2026-04-13
**Depends on**: `artifact-model-v0.1.md`, `subsystem-map-v0.1.md`
**Related**: `forge-v0.1.md`, `stiglab-*` (forthcoming), `ising-*` (forthcoming)

---

## 1. Purpose

Synodic is the **governance gate subsystem** of Onsager. It evaluates every action that touches an artifact — before dispatch, before state transition, before consumer routing, and during shaping sessions — and issues verdicts that are structurally unbypassable.

Synodic is not a policy editor. It is not a rules engine you program directly. It is not an authorization system. It is the **governance layer** of the factory: the subsystem that ensures nothing happens to an artifact that should not happen, and that everything that does happen leaves an auditable trace.

This document defines Synodic's responsibilities, its gate protocol, its rule system, the crystallization pipeline, escalation semantics, and its invariants. It does **not** specify API endpoints, database schemas, UI surfaces, or concrete rule evaluation algorithms — those are implementation concerns.

---

## 2. Responsibilities

Synodic does five things and only five things:

1. **Gate** — receive gate requests from Forge and Stiglab, evaluate them against the current rule set, and return binding verdicts
2. **Evaluate** — apply typed, versioned rules to gate requests, considering artifact state, factory context, and rule precedence
3. **Escalate** — when autonomous evaluation is insufficient, route decisions to a human or delegated system with configurable timeouts and conservative defaults
4. **Crystallize** — transform repeated decisions (human or AI) into static rules that govern future actions automatically
5. **Audit** — write every gate request, verdict, escalation outcome, and rule change to the factory event spine

Everything else is **out of scope**. Specifically:

- Synodic does **not** drive artifact state. Forge does.
- Synodic does **not** run agent sessions. Stiglab does.
- Synodic does **not** detect patterns or propose improvements. Ising does.
- Synodic does **not** authenticate users or manage permissions. That is AuthN/AuthZ, which is a separate concern.
- Synodic does **not** provide a UI for editing rules. A Console or admin surface may exist, but it is not part of Synodic.

Synodic is **the one place where governance verdicts are issued**. No other subsystem produces binding governance judgments. This single-gate constraint is how Onsager makes the structural promise that governance cannot be bypassed by the factory itself.

---

## 3. Mental model

```
                    ┌──────────────────────────────────────────┐
                    │                SYNODIC                   │
                    │                                          │
  GateRequest ────► │   ┌────────────┐      ┌──────────────┐   │
  (from Forge       │   │   Rule     │      │  Evaluation  │   │
   or Stiglab)      │   │   store    │ ───► │    engine    │   │
                    │   │ (versioned)│      └──────┬───────┘   │
                    │   └────────────┘             │           │
                    │                        ┌─────┴─────┐     │
                    │                        ▼           ▼     │
                    │                  ┌──────────┐ ┌────────┐ │
                    │                  │ Verdict  │ │Escalate│ │
                    │                  │ (sync)   │ │(async) │ │
                    │                  └─────┬────┘ └───┬────┘ │
                    │                        │         │       │
  GateVerdict ◄──── │ ◄──────────────────────┴─────────┘       │
                    │                                          │
                    │         ┌──────────────────┐             │
                    │         │  Crystallization  │             │
                    │         │     pipeline      │             │
                    │         └──────────────────┘             │
                    └──────────────────────────────────────────┘
```

Synodic's core path is synchronous and fast:

```
gate_request = receive(forge_or_stiglab)
rules = rule_store.applicable_rules(gate_request.context)
verdict = evaluation_engine.evaluate(gate_request, rules, factory_context)
if verdict == Escalate:
    park(gate_request, timeout, default_verdict)
    // async: human or delegate resolves
    verdict = await resolution_or_timeout()
emit_factory_event(gate_request, verdict)
return verdict
```

The synchronous path (Allow, Deny, Modify) must be fast — Forge and Stiglab are waiting. Escalation is the only path that introduces latency, and it is explicitly asynchronous with a timeout.

---

## 4. Gate points

Synodic is consulted at four gate points. The first three are driven by Forge; the fourth by Stiglab.

### 4.1 Pre-dispatch

Before Forge sends a `ShapingRequest` to Stiglab, Synodic evaluates whether the proposed shaping should proceed. Synodic sees the `shaping_intent`, the target artifact, and the current factory context.

**What Synodic may decide:**
- Allow — proceed with dispatch
- Deny(reason) — block the shaping; artifact stays in current state
- Modify(new_action) — proceed with a modified shaping intent (e.g., constrain scope, add safety requirements)
- Escalate(ctx) — park the decision for human review

### 4.2 State transition

Before Forge advances an artifact to a new lifecycle state, Synodic evaluates whether the transition is permitted. Synodic sees the proposed transition, the artifact's current state, its quality signals, and its lineage.

**What Synodic may decide:**
- Allow — proceed with the transition
- Deny(reason) — block the transition; artifact stays in current state
- Modify(new_action) — allow the transition but with modifications (e.g., attach a required follow-up, add quality signal requirements)
- Escalate(ctx) — park the decision for human review; common for `under_review → released` transitions

### 4.3 Consumer routing

Before Forge dispatches a `released` artifact to external consumers, Synodic evaluates the routing. Synodic sees the artifact, the target consumers, and the routing configuration.

**What Synodic may decide:**
- Allow — route to all declared consumers
- Deny(reason) — block all routing; artifact remains `released` but undelivered
- Modify(new_action) — route to a subset of consumers, or route with redactions
- Escalate(ctx) — park the routing decision for human review

### 4.4 Tool-level gating (Stiglab sessions)

During an active shaping session, Stiglab may consult Synodic before executing specific tool calls. This is the Stiglab → Synodic gated protocol defined in `subsystem-map-v0.1 §4.1`. Synodic sees the tool name, tool arguments, the session's artifact context, and the current rule set.

**What Synodic may decide:**
- Allow — tool call proceeds
- Deny(reason) — tool call is blocked; Stiglab may retry with different arguments or report failure
- Modify(new_action) — tool call proceeds with modified arguments (e.g., sanitized inputs)
- Escalate(ctx) — tool call is parked; session may continue other work or wait

Forge does not mediate tool-level gating. This is a direct Stiglab → Synodic interaction. Forge only sees the final `ShapingResult`.

---

## 5. The gate protocol

### 5.1 Contract

```
GateRequest (Forge or Stiglab → Synodic):
    request_id: ULID
    gate_point: PreDispatch | StateTransition | ConsumerRouting | ToolLevel
    context: GateContext
    proposed_action: ProposedAction
    artifact_id: Option<ArtifactId>
    session_id: Option<SessionId>
    timestamp: Timestamp

GateVerdict (Synodic → requester):
    request_id: ULID   // echoes the request
    verdict: Allow | Deny(reason) | Modify(new_action) | Escalate(ctx)
    rule_ids: [RuleId]  // which rules contributed to this verdict
    evaluated_at: Timestamp
    confidence: Option<float>  // for rules that involve judgment
```

### 5.2 Verdict semantics

**Allow** — the proposed action may proceed as-is. The requester is free to execute immediately.

**Deny(reason)** — the proposed action is blocked. The `reason` field is a structured explanation, not a free-text string. It identifies which rules triggered the denial and why.

**Modify(new_action)** — the proposed action is allowed only if transformed into `new_action`. The requester must use the modified version. If the requester cannot honor the modification, it must treat the verdict as a Deny.

**Escalate(ctx)** — Synodic cannot decide autonomously. The decision is parked and routed to a human or delegated system. The `ctx` field contains the information needed for the escalation handler to make an informed decision. See §7 for escalation semantics.

### 5.3 Protocol mode

Both Forge → Synodic and Stiglab → Synodic are **gated** protocols, as defined in `subsystem-map-v0.1 §3.2`. The verdict is binding. There is no override mechanism on the requester side. This is a structural property, not a policy choice.

### 5.4 Latency expectations

The synchronous path (Allow, Deny, Modify) should complete in low milliseconds. Synodic evaluates rules against in-memory state; it does not make external calls during synchronous evaluation. If a rule requires external data, that data must be pre-fetched into Synodic's evaluation context before the gate request arrives.

Escalation has no latency guarantee. It is explicitly asynchronous. The requester parks the decision and continues with other work.

---

## 6. Rule system

### 6.1 What a rule is

A rule is a **typed, versioned, auditable governance predicate**. It takes a `GateContext` and a `ProposedAction` and contributes to a verdict.

```
Rule:
    rule_id: RuleId
    version: int  // monotonically increasing
    type: Structural | Policy | Advisory
    gate_points: [GatePoint]  // which gate points this rule applies to
    condition: Condition  // what triggers the rule
    action: Allow | Deny(reason) | Modify(transform) | Escalate(ctx)
    priority: int  // higher-priority rules evaluated first
    enabled: bool
    created_at: Timestamp
    created_by: RuleSource  // human | crystallization | system
    rationale: string  // why this rule exists
```

### 6.2 Rule types

Rules come in exactly three types. Every rule belongs to exactly one type.

**Structural rules** — always enforced, cannot be disabled by operators, represent invariants the factory must never violate. Examples: "no artifact may be released without at least one quality signal," "no shaping may target an archived artifact." Structural rules are the factory's constitution. They are few in number and change rarely.

**Policy rules** — enforced by default, configurable per artifact kind or per owner. Operators may adjust thresholds or disable specific policy rules for specific contexts. Examples: "code artifacts require test coverage above 80% before release," "documents must pass a readability check before routing to external consumers." Policy rules are the factory's standard operating procedures.

**Advisory rules** — evaluated and logged but not enforced. Their verdicts are recorded in the audit trail but do not block actions. Examples: "flag artifacts that have been in_progress for more than 7 days," "note when horizontal lineage references more than 5 input artifacts." Advisory rules are the factory's soft warnings. They exist so Ising can observe patterns without Synodic blocking production.

### 6.3 Rule precedence

When multiple rules apply to the same gate request:

1. **Structural rules always win.** If any structural rule denies, the verdict is Deny regardless of other rules.
2. **Among non-structural rules, the most restrictive verdict wins.** Deny beats Modify beats Escalate beats Allow.
3. **Advisory rules do not affect the verdict.** They are recorded alongside the verdict but cannot override it.
4. **Priority breaks ties within the same type and verdict.** Higher-priority rules provide the reason string.

This precedence model is conservative by design. The factory defaults to caution. Loosening governance requires changing rules, not overriding verdicts.

### 6.4 Rule versioning

Every rule modification produces a new version. Old versions are retained for audit. The evaluation engine always uses the latest enabled version of each rule. Rule version history is append-only — no version is ever deleted or modified after creation.

When a rule is disabled, it remains in the rule store at its current version with `enabled: false`. When re-enabled, it resumes at the same version.

---

## 7. Escalation

### 7.1 When escalation happens

Synodic escalates when:

- A rule explicitly specifies `Escalate` as its action for a matching condition
- No rule covers the gate request (the "unknown territory" case — conservative default is to escalate rather than silently allow)
- Rule evaluation produces conflicting verdicts that the precedence model cannot resolve (should be rare given the strict precedence in §6.3, but possible during rule transitions)

### 7.2 Escalation mechanics

When Synodic escalates, it:

1. Emits a `synodic.escalation_started` factory event with the full gate context
2. Parks the gate request and returns `Escalate(ctx)` to the requester
3. The requester (Forge or Stiglab) parks the associated artifact or tool call
4. The escalation is routed to a human operator or delegated system via a configurable channel (Console queue, webhook, etc.)
5. The handler reviews the context and submits a resolution: Allow, Deny, or Modify
6. Synodic receives the resolution, emits `synodic.escalation_resolved`, and delivers the verdict to the original requester

### 7.3 Escalation timeouts

Every escalation has a configurable timeout. Timeouts are set per gate point and per artifact kind:

```
EscalationTimeout:
    gate_point: GatePoint
    artifact_kind: Option<ArtifactKind>  // None = applies to all kinds
    timeout_duration: Duration
    default_verdict: Deny(reason) | Modify(action)
```

**On timeout, the default verdict applies.** The default verdict is always conservative:

- Pre-dispatch timeout → Deny ("escalation timed out; shaping not dispatched")
- State transition timeout → Deny ("escalation timed out; artifact stays in current state")
- Consumer routing timeout → Deny ("escalation timed out; artifact not routed")
- Tool-level timeout → Deny ("escalation timed out; tool call blocked")

The default verdict is **never Allow**. A timed-out escalation cannot silently permit an action. This is the "timeout = deny" principle.

### 7.4 Escalation while parked

While an escalation is pending, the rest of the factory continues operating. Other artifacts are scheduled, other gate requests are evaluated. Escalation is non-blocking to the factory as a whole — only the specific artifact or tool call involved is parked.

This is consistent with Forge invariant #5 (`forge-v0.1 §10`): "an escalated decision does not freeze the entire scheduling loop."

---

## 8. Rule crystallization

### 8.1 The concept

Crystallization is how the factory learns. Repeated decisions — human escalation resolutions, Ising insights, operator corrections — become static rules that govern future actions automatically. This is the path from ad-hoc judgment to systematic governance.

Crystallization is Synodic's most important long-term capability. Without it, governance is either entirely hand-authored (does not scale) or entirely AI-judged (not trustworthy enough for high-stakes decisions). Crystallization produces a middle path: human decisions that compound into a growing body of codified governance.

### 8.2 The crystallization pipeline

```
Ising insight ──┐
                ├──► Proposed rule ──► Review ──► Approved rule ──► Gates future actions
Human decision ─┘
```

**Stage 1: Signal.** A crystallization candidate emerges from one of two sources:
- **Ising insight** — Ising observes a recurring pattern (e.g., "code artifacts shaped by session type X fail review 80% of the time") and proposes a rule via the factory event spine
- **Escalation resolution** — A human resolves an escalation, and the same kind of escalation has been resolved the same way N times (configurable threshold)

**Stage 2: Proposed rule.** Synodic creates a candidate rule with type `Policy` (crystallized rules are never automatically Structural — promoting to Structural requires explicit human decision). The candidate is tagged with its evidence: which insights or escalation resolutions it was derived from.

**Stage 3: Review.** A human or delegated system reviews the proposed rule. Review includes:
- Verifying the rule's condition matches the intended pattern
- Confirming the rule's action is appropriate
- Checking for false-positive risk
- Assessing interaction with existing rules

**Stage 4: Approved rule.** The rule enters the active rule set, versioned and enabled. From this point, it participates in gate evaluation like any other rule.

**Stage 5: Governing.** The rule gates future actions automatically. Its effectiveness is observable through the factory event spine — Ising can track whether the rule reduces the pattern it was designed to address.

### 8.3 Crystallization is not automatic

No rule enters the active set without review. Crystallization proposes; humans approve. This prevents two failure modes:

- **Runaway rulemaking** — the system invents rules faster than operators can understand them, burying the factory in governance noise
- **Spurious correlation** — a pattern that looks like a governance signal but is actually coincidence gets encoded as a permanent rule

The human-in-the-loop gate at Stage 3 is not a bottleneck to be optimized away. It is a feature. Governance rules that affect production at scale must be understood by the people responsible for that production.

### 8.4 Crystallization evidence

Every crystallized rule retains a link to its evidence — the specific insights, escalation resolutions, or operator decisions that led to its creation. This evidence chain is immutable and auditable. If someone asks "why does this rule exist?", the answer is always traceable to concrete factory history.

---

## 9. Factory event spine interaction

### 9.1 Events Synodic writes

Synodic emits the following factory events. This list is **authoritative**:

| Event | When |
|---|---|
| `synodic.gate_evaluated` | A gate request was evaluated and a verdict issued |
| `synodic.gate_denied` | A gate request was denied (subset of gate_evaluated, for easy filtering) |
| `synodic.gate_modified` | A gate request verdict was Modify (subset of gate_evaluated) |
| `synodic.escalation_started` | An escalation was initiated |
| `synodic.escalation_resolved` | An escalation was resolved (by human, delegate, or timeout) |
| `synodic.escalation_timed_out` | An escalation timed out and the default verdict was applied |
| `synodic.rule_proposed` | A crystallization candidate was created |
| `synodic.rule_approved` | A proposed rule was approved and entered the active set |
| `synodic.rule_disabled` | A rule was disabled |
| `synodic.rule_version_created` | A rule was modified, producing a new version |

### 9.2 Events Synodic reads

Synodic reads the factory event spine for **context during evaluation**. When evaluating a gate request, Synodic may consider:

- Recent `artifact.state_changed` events for the target artifact (to understand trajectory)
- Recent `forge.gate_denied` events (to avoid redundant evaluations)
- Recent `forge.shaping_returned` events (to assess quality trajectory)
- `ising.insight_*` events (to incorporate Ising observations into rule crystallization)

Synodic reads the spine selectively and on-demand. It is **not** a continuous consumer of the full spine — that is Ising's role. Synodic reads what it needs to evaluate the current gate request and no more.

---

## 10. Invariants

Synodic must maintain these invariants. Violation is a bug:

1. **No bypass** — Every gate point defined in §4 is consulted before the corresponding action proceeds. There is no operator override, no force flag, no debug bypass, no fast path that skips Synodic. This invariant is enforced structurally by Forge and Stiglab, not by Synodic itself — Synodic cannot prevent being skipped, but callers must never skip it.

2. **Verdict immutability** — Once a `GateVerdict` is issued and emitted to the factory event spine, it is never retracted, modified, or deleted. If a verdict was wrong, the correct response is to change the rule and re-submit the action — not to retroactively alter the verdict record.

3. **Rule versioning** — Every rule modification produces a new version. No version is ever deleted or overwritten. The complete history of every rule is always available for audit.

4. **Escalation timeout enforcement** — Every escalation has a timeout. No escalation may wait indefinitely. On timeout, the default verdict (which is always conservative) is applied. There is no "disable timeout" option.

5. **Audit completeness** — Every gate request, every verdict, every escalation start, every escalation resolution, every rule change, and every crystallization event is recorded on the factory event spine. There is no governance action that leaves no trace.

6. **Structural rule supremacy** — No policy or advisory rule can override a structural rule's verdict. If a structural rule denies, the verdict is Deny regardless of any other rule's evaluation.

7. **Conservative defaults** — In every ambiguous situation (unknown gate context, conflicting rules, timeout, evaluation failure), Synodic defaults to the more restrictive verdict. The factory fails closed, not open.

---

## 11. What Synodic explicitly does not do

To prevent scope creep, these are non-responsibilities:

- **Not a policy editor UI** — Synodic stores and evaluates rules. The interface for authoring, browsing, and managing rules belongs to the Console or an admin surface. Synodic exposes the rule store; it does not render it.
- **Not a rules engine you program directly** — Synodic's rule set evolves through crystallization, operator configuration, and system defaults. It is not a general-purpose rules engine where users write arbitrary predicates in a DSL. The rule model is deliberately constrained to typed conditions and structured actions.
- **Not an authorization system** — Synodic does governance ("should this action happen?"), not authentication ("who is this?") or authorization ("is this user allowed to do this?"). AuthN/AuthZ is a prerequisite that Synodic assumes is already resolved before a gate request arrives.
- **Not a compliance framework** — Synodic enforces factory-level governance. Regulatory compliance (SOC 2, GDPR, HIPAA) requires domain-specific rules and audit surfaces that are outside Synodic's scope. Synodic's audit trail may feed into compliance systems, but Synodic itself is not one.
- **Not an AI judge** — Synodic evaluates rules deterministically. If a governance decision requires AI judgment (e.g., "is this artifact's content safe?"), that judgment is produced by an external system and consumed by Synodic as a quality signal or a pre-computed context value. Synodic does not run inference during gate evaluation.
- **Not a second Ising** — Synodic does not observe the factory for patterns. It gates actions. If Synodic finds itself needing to "detect" or "analyze" production patterns, that work belongs in Ising, with results flowing through the crystallization pipeline.

---

## 12. Anti-patterns

If you catch yourself writing any of the following in Synodic's implementation, stop and reconsider:

- **"Forge skips Synodic for low-risk artifacts"** — violates invariant #1. There is no such thing as a low-risk artifact from Synodic's perspective. Risk assessment is Synodic's job, not the caller's.
- **"Synodic calls Claude to evaluate this rule"** — Synodic evaluates rules, it does not run inference. If AI judgment is needed, it must be pre-computed and available in the gate context before the request arrives.
- **"Synodic reads Stiglab's session database to check..."** — violates `subsystem-map-v0.1 §5.2`. Synodic sees gate requests and factory events. It does not access other subsystems' internal state.
- **"Let the operator override this Deny just this once"** — violates invariant #1 and the core governance promise. If the rule is wrong, fix the rule.
- **"Automatically promote this crystallization candidate — it has enough evidence"** — violates §8.3. Crystallization proposes; humans approve.
- **"Skip the escalation timeout in dev mode"** — violates invariant #4. Timeouts apply everywhere. Dev mode may have shorter timeouts, not absent timeouts.
- **"Delete this rule version — it was a mistake"** — violates invariant #3. Disable the rule and create a corrected version.
- **"Synodic blocks Forge from scheduling during an escalation"** — violates Forge's escalation non-blocking invariant (`forge-v0.1 §10 #5`). Escalations park individual decisions, not the factory.

---

## 13. Relationship to the synodic repo

The `onsager-ai/synodic` repository contains Synodic's implementation. As of this spec's writing, that repo has evolved from an earlier "harness" model (hook-based governance for individual Claude Code sessions) toward the factory-grade governance gate described here.

Key implementation concepts from the existing repo that map onto this spec:

| Existing concept | Maps to |
|---|---|
| L2 interception rules | Policy rules (§6.2) |
| Rule lifecycle (candidate → active → tuned → crystallized) | Crystallization pipeline (§8.2) |
| Bayesian confidence tracking (alpha/beta) | Evidence for crystallization (§8.4) |
| Post-session review model | Factory-level gate evaluation, refocused from session-level to artifact-level |
| Threat taxonomy | Structural and policy rule categories |
| Convergence detection | A concern Ising observes; Synodic consumes the result as context |

What changes from the existing repo: Synodic's gating scope expands from "tool calls within a single Claude Code session" to "every action that touches an artifact across the entire factory." The hook-based interception model remains valid as the implementation mechanism for tool-level gating (§4.4), but Synodic now also gates dispatch, state transitions, and consumer routing — responsibilities that did not exist in the pre-Onsager model.

---

## 14. Open questions

1. **Rule conflict resolution** — §6.3 defines a strict precedence model. Is this sufficient, or do we need an explicit conflict-resolution mechanism for cases where two policy rules of equal priority produce contradictory verdicts? v0.1 assumes the most restrictive wins, which is conservative but may be too blunt.

2. **Escalation routing** — §7.2 says escalations are routed to "a human operator or delegated system via a configurable channel." What are the concrete channel types? Console queue is obvious; webhook is likely. Are there others? How does Synodic know which channel to use for which escalation?

3. **Cross-subsystem rule visibility** — Should Forge and Stiglab be able to query Synodic's rule set to avoid submitting gate requests that will obviously be denied? This would reduce latency but creates coupling. v0.1 assumes no — callers submit requests and receive verdicts; they do not pre-filter.

4. **Crystallization threshold** — §8.2 mentions "the same kind of escalation has been resolved the same way N times." What is N? This is a tuning parameter, not an architectural decision. v0.1 does not specify it. The existing repo uses evidence-based thresholds (alpha/beta tracking); the same approach likely applies here.

5. **Advisory rule promotion** — Can an advisory rule be promoted to a policy rule based on evidence? This seems natural but is not defined in v0.1. The crystallization pipeline currently only creates new rules; it does not change the type of existing ones.

6. **Multi-Synodic coordination** — If two Synodic instances run in an HA pair, how do they avoid issuing conflicting verdicts for the same gate request? Requires either leader election or a shared rule-evaluation state. Not specified in v0.1.

7. **Synodic escalation deadlines** — Who configures per-transition timeouts? The artifact kind? The operator globally? Per-rule? This needs alignment with Forge (which raised the same question in `forge-v0.1 §13 #7`).

8. **Rule evaluation performance** — §5.4 requires low-millisecond synchronous evaluation. What is the acceptable upper bound on rule count before this becomes a latency concern? How does the rule store handle thousands of rules without degrading gate latency?

---

## 15. Decision log

| Date | Decision | Rationale |
|---|---|---|
| 2026-04-13 | Synodic verdicts are structurally unbypassable | Core of the governance story; inherited from vision §4 |
| 2026-04-13 | Three rule types: structural, policy, advisory | Matches the spectrum from hard invariant to soft signal |
| 2026-04-13 | Most restrictive verdict wins in conflicts | Conservative-by-default is safer than permissive-by-default |
| 2026-04-13 | Timeout always equals deny, never allow | A governance system that silently permits on timeout is not governance |
| 2026-04-13 | Crystallization requires human approval | Prevents runaway rulemaking and preserves operator agency |
| 2026-04-13 | Crystallized rules are never automatically Structural | Structural rules are constitutional; only humans may expand the constitution |
| 2026-04-13 | Synodic does not run AI inference during evaluation | Keeps gate latency deterministic; AI judgment is pre-computed context |
| 2026-04-13 | Verdicts are immutable once issued | Audit integrity depends on never rewriting history |
| 2026-04-13 | Four gate points: pre-dispatch, state transition, consumer routing, tool-level | Covers every moment an artifact is touched; no gaps |
| 2026-04-13 | Forge → Synodic and Stiglab → Synodic are both gated protocols | Consistent with subsystem-map §4.1; governance is the same regardless of caller |
