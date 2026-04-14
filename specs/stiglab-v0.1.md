# Stiglab — Onsager v0.1

**Status**: Draft
**Owner**: Marvin
**Last updated**: 2026-04-13
**Depends on**: `artifact-model-v0.1.md`, `subsystem-map-v0.1.md`
**Related**: `forge-v0.1.md`, `synodic-v0.1.md`, `ising-v0.1.md`

---

## 1. Purpose

Stiglab is the **distributed shaping runtime** of Onsager. It runs the agent sessions that actually shape artifacts. When Forge says "shape artifact X like this," Stiglab allocates a node, spawns agents, manages the session's internal state, and returns a shaping result.

Stiglab is not a scheduler. It is not a workflow engine. It is not a governance system. It is a **session execution substrate** — the layer that turns a `ShapingRequest` into a `ShapingResult` by running agents against real codebases, documents, and data. Everything else is someone else's job.

This document defines Stiglab's responsibilities, its session model, its protocols with Forge and Synodic, its node management model, and its invariants. It does **not** specify database schemas, API endpoints, or agent prompt engineering — those are implementation concerns.

---

## 2. Responsibilities

Stiglab does five things and only five things:

1. **Accept** — receive `ShapingRequest`s from Forge, validate them, and acknowledge receipt
2. **Allocate** — select a node from the pool, assign the request to it, and track capacity
3. **Execute** — spawn an agent session on the allocated node, manage its lifecycle, handle internal events
4. **Gate mid-session** — consult Synodic for tool-level governance decisions during session execution
5. **Return** — package the session outcome into a `ShapingResult` and deliver it to Forge

Everything else is **out of scope**. Specifically:

- Stiglab does **not** decide what to shape. Forge does.
- Stiglab does **not** evaluate governance policy. Synodic does.
- Stiglab does **not** store artifact state. Forge does.
- Stiglab does **not** detect factory-wide patterns. Ising does.
- Stiglab does **not** store artifact content. External systems do. Stiglab produces a `content_ref` pointing to where the content landed.
- Stiglab does **not** manage horizontal lineage. Forge does, using the `session_id` Stiglab provides.

Stiglab **owns the concept of session**. No other subsystem creates, mutates, or terminates sessions. Forge references session IDs for vertical lineage; Synodic sees session IDs on tool-level gate consultations for audit. Neither reaches into session state. This boundary is defined in `subsystem-map-v0.1 §5.3` and is non-negotiable.

---

## 3. Mental model

```
         ┌────────────────────────────────────────────────────┐
         │                     STIGLAB                        │
         │                                                    │
         │   ┌───────────┐      ┌───────────────────────┐    │
         │   │  Accept   │ ───► │   Allocate to node    │    │
         │   │  request  │      │   from pool           │    │
         │   └───────────┘      └───────────────────────┘    │
         │                              │                     │
         │                              ▼                     │
         │                      ┌───────────────────────┐    │
         │                      │   Spawn agent         │    │
         │                      │   session on node     │    │
         │                      └───────────────────────┘    │
         │                              │                     │
         │                    ┌─────────┴──────────┐         │
         │                    ▼                    ▼          │
         │            ┌──────────────┐    ┌──────────────┐   │
         │            │  Internal    │    │   Synodic    │   │
         │            │  event       │    │   tool-gate  │   │
         │            │  stream      │    │   consults   │   │
         │            └──────────────┘    └──────────────┘   │
         │                    │                    │          │
         │                    └─────────┬──────────┘         │
         │                              ▼                     │
         │                      ┌───────────────────────┐    │
         │                      │   Upgrade selected    │    │
         │                      │   events to factory   │    │
         │                      │   spine               │    │
         │                      └───────────────────────┘    │
         │                              │                     │
         │                              ▼                     │
         │                      ┌───────────────────────┐    │
         │                      │   Package and return  │    │
         │                      │   ShapingResult       │    │
         │                      └───────────────────────┘    │
         └────────────────────────────────────────────────────┘
```

The core execution path for a single shaping request is:

```
request = forge.receive_shaping_request()
node = pool.allocate(request.constraints)
session = node.spawn_session(request)
loop:
    event = session.next_event()
    if event.requires_tool_gate:
        verdict = synodic.consult(event.tool_context)
        if verdict.deny:
            session.block_tool(event)
    if event.is_governance_significant:
        spine.upgrade(event)
    if session.is_terminal:
        break
result = session.package_result()
pool.release(node, session)
spine.write(session_lifecycle_event)
forge.return_result(result)
```

This is deliberately serial per session. Stiglab runs many sessions concurrently across its node pool, but each individual session is a single sequential execution with one agent runtime.

---

## 4. The session concept

### 4.1 What a session is

A **session** is a bounded execution of one or more agents against one artifact, under one `ShapingRequest`, producing one `ShapingResult`. It is the fundamental unit of work inside Stiglab.

Sessions have four properties:

- **Bounded** — every session starts and ends. There are no perpetual sessions. The `deadline` from the `ShapingRequest` is enforced; sessions that exceed it are aborted.
- **Artifact-bound** — every session is bound to exactly one artifact via the `artifact_id` in the `ShapingRequest`. A session cannot shape two artifacts. An artifact can be shaped by many sessions (sequentially, per `forge-v0.1 §13.1`).
- **Internally rich** — inside a session, dozens of events happen per second: tool calls, tool results, agent reasoning, subagent spawns, file reads, file writes. All of this is session-internal. It does not leave Stiglab unless explicitly upgraded.
- **Externally opaque** — to Forge, a session is a black box identified by a `session_id`. Forge knows when it started, when it ended, and what it produced. It does not know what happened inside. This opacity is deliberate and structural.

### 4.2 Session identity

Stiglab assigns a `session_id` (ULID) when a session is created. This ID:

- Is globally unique and never reused
- Is returned to Forge in the `ShapingResult` for vertical lineage
- Is included in Synodic tool-gate consultations for audit correlation
- Is included in any factory events that Stiglab upgrades to the spine

No other subsystem creates session IDs. This is the invariant from `subsystem-map-v0.1 §5.3`.

### 4.3 Session lifecycle

```
created → dispatched → running → completed
                         │  ↑        │
                         │  │        ├── failed
                         │  │        │
                         ▼  │        └── aborted
                    waiting_input
```

| State | Meaning |
|---|---|
| `created` | Session record exists; no node allocated yet |
| `dispatched` | Node allocated; agent subprocess not yet started |
| `running` | Agent is actively executing |
| `waiting_input` | Agent has paused and is awaiting external input (human or system) |
| `completed` | Agent finished successfully; `ShapingResult` has outcome `Completed` |
| `failed` | Agent encountered an unrecoverable error; `ShapingResult` has outcome `Failed` |
| `aborted` | Session was terminated externally (deadline exceeded, Synodic denial, Forge cancellation); `ShapingResult` has outcome `Aborted` |

`waiting_input` is a distinguishing feature of AI agent sessions that traditional task runners lack. An agent may pause mid-execution to request human clarification, permission approval, or additional context. Stiglab must detect this state reliably — not by pattern-matching stdout, but through the agent runtime's structured event stream.

Terminal states (`completed`, `failed`, `aborted`) are irreversible. A session that reaches a terminal state never transitions again.

---

## 5. Protocols

### 5.1 Forge to Stiglab (imperative)

Forge dispatches shaping work to Stiglab. This is a one-way imperative protocol as defined in `subsystem-map-v0.1 §3.1`. Stiglab must attempt the command. Stiglab may fail, but may not refuse on policy grounds — policy enforcement is Synodic's job.

The contract is defined in `forge-v0.1 §5.2`:

```
ShapingRequest (Forge → Stiglab):
    request_id: ULID
    artifact_id: ArtifactId
    target_version: int
    shaping_intent: structured payload
    inputs: [ArtifactRef]
    constraints: [Constraint]
    deadline: Option<timestamp>

ShapingResult (Stiglab → Forge):
    request_id: ULID
    outcome: Completed | Failed | Partial | Aborted
    content_ref: Option<ExternalURI>
    change_summary: string
    quality_signals: [QualitySignal]
    session_id: SessionId
    duration_ms: int
    error: Option<ErrorDetail>
```

**Idempotency**: if Forge sends the same `request_id` twice (retry after timeout), Stiglab deduplicates. If the session is already in progress, Stiglab returns the in-progress status. If the session has completed, Stiglab returns the cached `ShapingResult`. This honors `forge-v0.1 §10.6`.

**Cancellation**: Forge may cancel an in-flight `ShapingRequest` by sending a cancellation keyed to `request_id`. If the session is still running, Stiglab aborts it and returns a `ShapingResult` with outcome `Aborted`. If the session has already completed, the cancellation is a no-op.

### 5.2 Stiglab to Synodic (gated)

During session execution, Stiglab consults Synodic for tool-level governance decisions. This is a gated protocol as defined in `subsystem-map-v0.1 §3.2`. Synodic's verdict is binding — Stiglab has no override mechanism.

```
ToolGateRequest (Stiglab → Synodic):
    session_id: SessionId
    artifact_id: ArtifactId
    tool_name: string
    tool_input: structured payload
    context: ToolGateContext  // working directory, recent actions, etc.

ToolGateVerdict (Synodic → Stiglab):
    verdict: Allow | Deny(reason) | Modify(modified_input)
```

Gate consultations happen **inline** during session execution. The session blocks on the verdict. This means Synodic's latency directly impacts session throughput — a design cost accepted in exchange for governance integrity.

**When to consult Synodic**: Not every tool call requires a gate consultation. Stiglab maintains a **gate policy** (configured by Synodic) that specifies which tool patterns require gating. Read-only operations against the working directory typically pass through ungated. Destructive operations, network calls, and privilege escalations are always gated.

**Denied tools**: When Synodic denies a tool call, Stiglab does not terminate the session. It blocks the specific tool and lets the agent runtime handle the denial (most runtimes will attempt an alternative approach). If the agent cannot make progress after a denied tool, the session will eventually fail or be aborted by deadline — this is correct behavior.

### 5.3 Stiglab to factory event spine

Stiglab writes session lifecycle events to the factory event spine. These are the **only** events Stiglab puts on the spine — session-internal events stay internal.

| Event | When |
|---|---|
| `stiglab.session_created` | Session record created |
| `stiglab.session_dispatched` | Node allocated, agent about to spawn |
| `stiglab.session_running` | Agent subprocess started |
| `stiglab.session_completed` | Session finished successfully |
| `stiglab.session_failed` | Session terminated with error |
| `stiglab.session_aborted` | Session externally terminated |
| `stiglab.event_upgraded` | A session-internal event was upgraded to factory significance (see §6) |
| `stiglab.node_registered` | New node joined the pool |
| `stiglab.node_deregistered` | Node left the pool (graceful or timeout) |
| `stiglab.node_heartbeat_missed` | Node failed to heartbeat within the expected window |

This list is authoritative. Adding new event types is a versioned change to this spec.

---

## 6. Event upgrading

This is the critical concept from `onsager-vision-v0.1 §6`.

### 6.1 The two layers

Inside a session, events are high-frequency and private: tool calls, tool results, agent reasoning tokens, file reads, file writes, subagent spawns. These are useful for debugging a specific session but meaningless to the factory at large.

On the factory event spine, events are low-frequency and shared: artifact state changes, gate verdicts, session lifecycle transitions. These are what Synodic and Ising consume to govern and improve the factory.

**Upgrading** is the act of promoting a session-internal event to a factory event because it has governance or improvement significance.

### 6.2 What gets upgraded

The decision of what to upgrade is governed by upgrade rules. v0.1 ships with a static set:

| Internal event | Upgraded to | Trigger |
|---|---|---|
| Tool call denied by Synodic | `stiglab.event_upgraded` with `kind: tool_denied` | Always — every Synodic denial is governance-significant |
| Destructive filesystem operation | `stiglab.event_upgraded` with `kind: destructive_op` | `rm -rf` outside working directory, writes to system paths |
| Network egress to unexpected host | `stiglab.event_upgraded` with `kind: network_egress` | Outbound connection to a host not in the allowed list |
| Session exceeded token budget | `stiglab.event_upgraded` with `kind: budget_exceeded` | Token count crosses the configured threshold |
| Agent requested escalation | `stiglab.event_upgraded` with `kind: agent_escalation` | Agent explicitly signals it cannot proceed without human input |

Future versions will make upgrade rules configurable through Synodic's rule pipeline. In v0.1, the rules are compiled in.

### 6.3 The upgrade ratio

The ratio of session-internal events to upgraded factory events is a health signal:

- **Ratio too high** (many internal events per upgrade) — the factory may be under-governed. Sessions are doing significant work without governance visibility.
- **Ratio too low** (upgrades approach 1:1 with internal events) — the factory is over-reporting. The governance layer is drowning in noise, which is the exact failure mode the two-layer architecture exists to prevent.

Stiglab tracks this ratio per session and per node. Ising consumes the aggregated ratio as one of its observation inputs. The healthy range is not specified in v0.1 — it will emerge from real workload observation.

### 6.4 Upgrading discipline

The upgrade boundary is a **one-way gate**. An event that has been upgraded cannot be "downgraded" back to internal. An event that stays internal cannot be retroactively read by Synodic or Ising — they would need to request session debug access through a separate (TBD) mechanism.

This discipline is what keeps the factory event spine meaningful and tractable at scale.

---

## 7. Node management

### 7.1 What a node is

A **node** is a machine (physical, virtual, or container) that runs a Stiglab agent process and can host agent sessions. Each node:

- Registers with the Stiglab control plane via WebSocket
- Declares its capacity (maximum concurrent sessions)
- Sends periodic heartbeats
- Receives session dispatch instructions
- Reports session lifecycle events back to the control plane

### 7.2 Node lifecycle

```
registered → active → draining → deregistered
                │                      ▲
                └── (heartbeat miss) ──┘
                         stale
```

| State | Meaning |
|---|---|
| `registered` | Node connected and announced capacity; not yet assigned work |
| `active` | Node is available for session dispatch |
| `draining` | Node is finishing existing sessions but accepting no new ones (graceful shutdown) |
| `deregistered` | Node has disconnected or been removed from the pool |
| `stale` | Node has missed heartbeats beyond the configured threshold; sessions on it are suspect |

### 7.3 Capacity tracking

Each node reports `max_sessions` on registration and `active_sessions` in each heartbeat. The control plane maintains a capacity view:

```
available_capacity(node) = node.max_sessions - node.active_sessions
```

Session allocation uses this view. A node with `available_capacity == 0` is not eligible for dispatch.

### 7.4 Node selection

When allocating a session, Stiglab selects a node from the pool based on:

1. **Available capacity** — node must have room
2. **Constraint matching** — the `ShapingRequest.constraints` may specify required capabilities (e.g., specific agent runtime, GPU, specific repository access)
3. **Locality** — prefer nodes that already have the relevant working directory warm (caches, git clones)
4. **Load balancing** — among eligible nodes, prefer the least loaded

v0.1 uses a simple least-loaded-first strategy with constraint filtering. Sophisticated placement algorithms are deferred.

### 7.5 Stale node recovery

If a node misses heartbeats beyond the threshold (default: 3 consecutive misses at 10-second intervals):

1. Node is marked `stale`
2. All sessions on the node are marked `failed` with error `node_stale`
3. Forge receives `ShapingResult` with outcome `Failed` for each affected session
4. Forge may re-decide and re-dispatch the affected artifacts

This is a blunt recovery strategy. v0.1 does not attempt session migration between nodes.

---

## 8. Agent runtime abstraction

### 8.1 Stiglab is not an agent framework

Stiglab runs **on top of** agent runtimes. It does not reinvent the agent layer. The agent runtime handles prompt construction, tool orchestration, context management, and model interaction. Stiglab handles session lifecycle, node management, event streaming, and governance integration.

This is the boundary stated in `onsager-vision-v0.1 §7`: "Onsager runs on top of agent runtimes through Stiglab. It does not reinvent the agent layer."

### 8.2 Runtime adapter interface

Each supported agent runtime is accessed through an adapter:

```
trait RuntimeAdapter:
    fn spawn(config: SessionConfig) -> SessionHandle
    fn send_input(handle: &SessionHandle, input: string)
    fn stream_events(handle: &SessionHandle) -> EventStream
    fn abort(handle: &SessionHandle)
```

The adapter translates between Stiglab's session model and the runtime's specific CLI invocation, event format, and lifecycle signals.

### 8.3 v0.1 scope: Claude Code only

v0.1 supports exactly one runtime: **Claude Code** via its `--output-format stream-json` mode. The adapter:

- Spawns Claude Code as a subprocess with structured NDJSON output
- Parses the event stream for tool calls, text output, and lifecycle transitions
- Detects `waiting_input` state through the runtime's structured events (not stdout pattern matching)
- Supports `--permission-mode bypassPermissions` for non-interactive execution, with Synodic gate consultations replacing the interactive permission model

Additional runtimes (Gemini CLI, Codex, custom) are explicitly deferred to post-v0.1. The adapter interface is designed to accommodate them, but no second implementation exists yet.

### 8.4 Relationship between Synodic gating and runtime permissions

Agent runtimes have their own permission models (e.g., Claude Code's `--permission-mode`). In Onsager, these are **bypassed** in favor of Synodic's governance layer. The agent runs with full permissions at the runtime level; Stiglab intercepts tool calls that require governance and routes them through the Synodic gate protocol (§5.2) before allowing execution.

This means Onsager's governance is **runtime-agnostic**. The same Synodic rules apply regardless of which agent runtime Stiglab uses. The runtime's built-in permissions are an implementation detail that Stiglab overrides.

---

## 9. Shaping semantics

### 9.1 What "shaping" means

**Shaping** is the act of modifying or creating artifact content according to a `shaping_intent`. It is the core verb of Stiglab. When a session shapes an artifact, it:

1. Reads the `shaping_intent` from the `ShapingRequest` — a structured description of what the shaping should accomplish
2. Reads the `inputs` — references to other artifacts that serve as horizontal lineage (source material, prior versions, specifications)
3. Executes agent actions — tool calls, code generation, document editing, API interactions — guided by the intent
4. Produces outputs:
   - `content_ref` — pointer to where the shaped content now lives (Git commit, S3 object, etc.)
   - `change_summary` — semantic description of what changed from the previous version
   - `quality_signals` — self-assessed quality metrics (test results, lint output, coverage, etc.)

### 9.2 Shaping intent interpretation

The `shaping_intent` is a structured payload whose schema varies by artifact `kind`. Stiglab translates shaping intent into agent instructions:

- For `code` artifacts: the intent might specify "implement feature X," "fix bug Y," "refactor module Z." Stiglab translates this into a prompt for the agent runtime, including relevant context from `inputs`.
- For `document` artifacts: the intent might specify "draft section on topic X using source Y." Same translation pattern.
- For `report` artifacts: the intent might specify "generate analysis of dataset X with methodology Y."

Stiglab does **not** define the shaping intent schema — that is Forge's responsibility (derived from the artifact `kind` system in `artifact-model-v0.1 §5`). Stiglab only consumes it.

### 9.3 Quality signal production

After shaping, the session produces quality signals by running kind-appropriate checks:

- `code`: run tests, linters, type checkers against the shaped output
- `document`: run readability checks, completeness validation against the intent
- `report`: run data integrity checks, format validation

These signals are included in the `ShapingResult` and become part of the artifact's quality history when Forge records them.

Stiglab does **not** evaluate whether quality signals are sufficient for state advancement — that is Forge's decision, informed by Synodic's gate verdicts.

---

## 10. Invariants

Stiglab must maintain these invariants. Violation is a bug:

1. **Session-to-artifact binding** — every session is bound to exactly one artifact. A session cannot shape multiple artifacts. This is enforced at session creation time from the `ShapingRequest.artifact_id`.

2. **No orphan sessions** — every session must be traceable to a `ShapingRequest` from Forge. Stiglab never creates sessions spontaneously. If a session exists without a corresponding request, it is a system bug.

3. **Session identity ownership** — only Stiglab creates, assigns, and manages session IDs. No other subsystem generates session IDs or mutates session state. This is the invariant from `subsystem-map-v0.1 §5.3`.

4. **Event upgrading discipline** — session-internal events do not leak to the factory event spine except through the explicit upgrade mechanism (§6). No bulk export, no "debug mode" that dumps internal events to the spine, no backdoor.

5. **Synodic gate honoring** — when Synodic denies a tool call during a session, Stiglab blocks that tool unconditionally. There is no force flag, no bypass, no "the agent really needs this" override. This is the structural governance guarantee from `subsystem-map-v0.1 §5.1`.

6. **Deadline enforcement** — if a `ShapingRequest` specifies a deadline, Stiglab must abort the session when the deadline is exceeded. Late results are not delivered as `Completed`; they are `Aborted` with error `deadline_exceeded`.

7. **No artifact state mutation** — Stiglab never writes to artifact state. It produces a `ShapingResult` which Forge uses to advance the artifact. Even when shaping obviously succeeded, Stiglab does not "helpfully" update the artifact directly. This is the single-writer invariant from `artifact-model-v0.1 §8`.

8. **Idempotent request handling** — duplicate `request_id`s from Forge produce the same `ShapingResult`, not duplicate sessions. At-most-once session creation per request.

9. **Terminal state irreversibility** — a session that reaches `completed`, `failed`, or `aborted` never transitions to any other state. Terminal is terminal.

10. **Node capacity honoring** — Stiglab never dispatches a session to a node that has reported zero available capacity. Over-subscription is a system bug.

---

## 11. What Stiglab explicitly does not do

To prevent scope creep, these are non-responsibilities:

- **Not a workflow engine** — Stiglab does not chain sessions into pipelines or DAGs. If artifact A depends on artifact B, Forge handles the sequencing through horizontal lineage. Stiglab shapes one artifact at a time per session.
- **Not a content store** — Stiglab produces `content_ref` pointers. It does not persist artifact content in its own storage. Content lands in external systems (Git, S3, etc.) as a side effect of agent execution.
- **Not a governance system** — Stiglab enforces Synodic verdicts but does not evaluate policy. It does not decide what is safe or appropriate — it asks Synodic and obeys.
- **Not an artifact registry** — Stiglab does not know the full artifact model. It receives an `artifact_id` and a `shaping_intent`; it returns a `ShapingResult`. The artifact's lifecycle, state machine, and lineage graph are Forge's concern.
- **Not a metrics aggregator** — Stiglab tracks per-session and per-node metrics for its own allocation decisions. Factory-wide analysis is Ising's job.
- **Not an agent framework** — Stiglab does not implement tool orchestration, prompt construction, or context management. Agent runtimes do that. Stiglab wraps them.

---

## 12. Open questions

Left for subsequent specs or implementation discovery. Not answered in this version:

1. **Session migration** — if a node goes stale mid-session, can the session be resumed on another node? v0.1 says no — the session fails and Forge re-dispatches. But for long-running sessions, re-execution from scratch is expensive. Checkpoint-and-resume across nodes would reduce waste but adds significant complexity.

2. **Multi-agent sessions** — can a single session spawn multiple agents (e.g., a coding agent and a review agent) collaborating on the same artifact? v0.1 assumes one agent per session. Multi-agent coordination within a session is a runtime-specific concern that the adapter interface does not yet model.

3. **Session warm-up** — can Stiglab pre-warm sessions (clone repos, load context) before Forge dispatches work? This would reduce session latency but requires Stiglab to speculate about future work, which blurs the Forge/Stiglab boundary.

4. **Input injection** — when a session enters `waiting_input`, who provides the input? Forge? A human operator via the Console? An automated system? v0.1 does not specify the `waiting_input` resolution path. Sessions that enter this state will eventually be aborted by deadline.

5. **Session cost tracking** — should Stiglab track token usage, API cost, and compute time per session? This data is valuable for Ising and for billing, but the accounting model is unspecified.

6. **Upgrade rule configurability** — v0.1 ships with static upgrade rules (§6.2). When and how these become configurable through Synodic's rule pipeline needs design work.

7. **Adapter hot-swap** — can a node switch from Claude Code to a different runtime without deregistering and re-registering? v0.1 assumes each node is configured with a single runtime at startup.

8. **Gate policy distribution** — how does Stiglab receive and update the gate policy that determines which tool calls require Synodic consultation (§5.2)? Push from Synodic? Pull on session start? Cached with TTL?

---

## 13. Decision log

| Date | Decision | Rationale |
|---|---|---|
| 2026-04-13 | Session identity is owned exclusively by Stiglab | Preserves the internal/factory event layer separation; inherited from subsystem-map §5.3 |
| 2026-04-13 | v0.1 supports Claude Code only | One runtime done well beats three done poorly; adapter interface is ready for more |
| 2026-04-13 | Runtime permissions bypassed in favor of Synodic gating | Governance must be runtime-agnostic; cannot depend on each runtime's permission model |
| 2026-04-13 | Event upgrade rules are static in v0.1 | Configurability adds complexity; learn the right rules from real workloads first |
| 2026-04-13 | No session migration on node failure | Checkpoint-and-resume is complex; fail-and-re-dispatch is simple and correct |
| 2026-04-13 | One agent per session in v0.1 | Multi-agent coordination is a research problem; do not let it block the core architecture |
| 2026-04-13 | Stale node recovery fails all sessions on that node | Blunt but correct; sophisticated recovery deferred |
| 2026-04-13 | Sessions that enter waiting_input are aborted by deadline | Input injection path is unspecified in v0.1; this prevents indefinite hangs |
| 2026-04-13 | Stiglab never writes artifact state | Single-writer invariant inherited from artifact-model; even obvious success goes through Forge |
| 2026-04-13 | Tool-gate consultations block the session inline | Latency cost accepted for governance integrity; async gating would create a gap where ungoverned tool calls execute |
