# ADR 0008 — Portal owns the agent control plane

- **Status**: Proposed
- **Date**: 2026-05-09
- **Identity impact**: no
- **Tracking issues**: #291 (implementation spec). Amends ADR
  0006 (closes the stiglab carve-out for `/agent/ws`); refines
  ADR 0004's amendment by removing the remaining clause-1
  exception.
- **Supersedes**: amends the rule clause of ADR 0006 (the
  `stiglab owns /agent/ws` exception).
- **Superseded by**: none

## Context

ADR 0004's 2026-04-30 amendment named portal as the owner of
clause 1 — the external HTTP boundary itself — with no exceptions.
ADR 0006 then carved out one exception in its rule statement:
*portal owns `/api/*`; **stiglab owns `/agent/ws`***. The carve-out
was a pragmatic compromise. `/agent/ws` is the agent ↔ stiglab
control plane: registered nodes (hosts running the Claude Code CLI
in agent mode) connect to stiglab over WebSocket to receive task
dispatch and report status. Moving the route in ADR 0006's PR
would have expanded scope significantly; carving it out let the
dispatcher work land cleanly.

The 2026-05-09 review of ADR 0006 (immediately after the merge of
PR #284) surfaced that the compromise is structurally wrong:

- ADR 0004's amendment is unambiguous: clause 1 is owned by
  portal. The stiglab exception is a clause-1 violation that ADR
  0006 effectively legitimized.
- Stiglab is a *factory* subsystem (alongside forge, synodic,
  ising). The architectural intent — restated across the CLAUDE.md
  "Architecture" section, ADR 0001, and ADR 0004 — is that factory
  subsystems have no external presence at all.
- The user's mental model (the external boundary is portal, full
  stop) is the cleaner architecture; the carve-out muddies it and
  makes the seam-rule story harder to teach.

`/agent/ws` itself is **not dashboard-facing**. The dashboard does
not call it — confirmed by grepping `apps/dashboard/src/` for
`agent/ws`. The protocol (defined in `crates/stiglab/src/core/
protocol.rs` as `AgentMessage` / `ServerMessage`) carries node
registration (`Register`), task dispatch (server → agent), and
result/status reporting (agent → server). The default URL the
agent CLI connects to is `ws://localhost:3000/agent/ws` (see
`crates/stiglab/src/main.rs` and `crates/stiglab/src/agent/
config.rs`). It is a real external surface — just one with a
narrow caller set (Onsager's own agent CLI, today co-located in
the Railway container; future deployments may put nodes
elsewhere).

The 2026-05-09 conversation considered three resolution shapes:
WebSocket proxy at portal (Option 1), HTTP polling + spine events
(Option 2), and a hybrid where the WS terminates at portal but
every message is also a spine event (Option 3). Option 1 is the
chosen path: smallest change, closes the architectural hole,
defers the bigger spine-as-truth question to a future ADR if it
ever pays for itself.

## Decision

Portal hosts `/agent/ws` as a transparent WebSocket proxy. The
agent CLI's URL is unchanged (`wss://host/agent/ws`); only the
process terminating the WebSocket changes.

The flow:

1. Agent CLI connects to `wss://host/agent/ws` — same URL as
   today.
2. Caddy reverse-proxies to portal (instead of stiglab). Both
   the dev `deploy/dev/Caddyfile` and the production Caddyfile
   from #283's implementation update.
3. Portal's `/agent/ws` handler accepts the WebSocket upgrade and
   authenticates the connection. v1 keeps the current auth model
   for the move to be purely structural; tightening to PAT or
   session-based auth is a follow-up.
4. Portal opens a backend WebSocket on loopback to stiglab
   (`ws://127.0.0.1:3000/agent/ws-internal`). Stiglab binds this
   server to `127.0.0.1` only; it is unreachable from outside the
   container.
5. Portal forwards bytes bidirectionally between the two
   WebSockets — two `tokio` tasks, one per direction. Either
   side closing closes the other; reconnection is the agent's
   existing responsibility.

After this ADR lands, the rule statement from ADR 0006's Decision
section is amended:

> The externally-reachable process is the edge dispatcher. The
> Onsager subsystems (`portal`, `stiglab`, `synodic`, `forge`,
> `ising`) are reachable only through the dispatcher's routing
> config. **Portal owns every external route; no subsystem
> carve-outs.** Stiglab's `/agent/ws` becomes a portal-owned route
> that proxies to stiglab on loopback.

Stiglab continues to own the WebSocket *protocol* — the
`AgentMessage` / `ServerMessage` enum, node registration, task
dispatch logic, the agent-runtime state. That is internal
agent-runtime concern. Stiglab just stops accepting connections
from anything except portal.

Cross-cutting alignment with prior ADRs:

- **ADR 0004** (clause 1 owned by portal): no remaining exception.
  The amendment is honored without exception.
- **ADR 0006** (dispatcher is the externally-reachable process):
  unchanged. The dispatcher still routes by path; the path now
  resolves to portal for both `/api/*` and `/agent/ws`.
- **ADR 0001** (spine for internal coordination): preserved.
  Portal-to-stiglab on loopback is *internal*; it is not
  cross-subsystem HTTP in the architectural sense (the WebSocket
  is the same wire format both sides already speak; portal is a
  transport-layer proxy, not a protocol translator).
- **ADR 0007** (MCP + skills as the public contract): unaffected.
  Agent-runtime control is a different concern from AI-runtime
  control (the public MCP surface).

## Rejected alternatives

- **Replace WebSocket with HTTP + spine events** (Option 2 from
  the 2026-05-09 conversation). Node polls portal over HTTP for
  tasks; reports status and results via POST; portal turns each
  request into a spine event; stiglab subscribes. Heaviest of the
  three options. Forces a real protocol redesign — partial-failure
  handling, reconnection semantics, dead-node detection all
  rebuilt on top of spine TTLs and event ordering. Rejected as
  over-engineering for the immediate goal of closing the clause-1
  hole. Filed as a possible follow-up if streaming proves to be
  the wrong long-term shape.
- **Hybrid: WebSocket at portal, every message also a spine event**
  (Option 3 from the conversation). More architecturally pure (the
  spine becomes the single source of truth for agent-runtime
  state) but adds spine event volume proportional to agent message
  rate. Rejected for v1: closing the clause-1 hole does not
  require it; spine-as-truth is its own architectural commitment
  that deserves its own ADR if and when it pays for itself.
- **Keep the carve-out and document the rationale.** The user
  explicitly rejected this in the 2026-05-09 review. The
  compromise was the wrong call in ADR 0006; this ADR exists
  precisely to undo it.
- **Move the entire WebSocket implementation to portal** — the
  protocol, node registration, task dispatch, all of it. Rejected:
  that puts agent-runtime concern in the wrong subsystem. Stiglab
  is the agent-session orchestrator; the protocol stays there.
  Portal is a thin transport-layer proxy.
- **Bind stiglab to loopback and trust the dispatcher to gate.**
  Caddy could reverse-proxy `/agent/ws` to stiglab on loopback —
  cosmetically the WebSocket is "behind" the dispatcher. Rejected
  because the route handler still lives in stiglab's source code:
  clause-1 ownership is about *which subsystem owns the route*,
  not just which process binds the public port. The user's
  pushback was specifically on this code-level ownership.

## Consequences

### Positive

- **Stiglab is fully internal at both layers.** Process-level
  (loopback bind, per ADR 0006) AND code-level (no public route
  handlers). The factory subsystem boundary is clean. The seam
  rule's amendment is honored without exception.
- **Portal owns 100% of the external HTTP surface.** No clause-1
  carve-outs. The seam rule is mechanical, not subject to
  case-by-case discretion.
- **Future deployment topologies inherit a consistent shape.**
  Multi-host, different cloud, self-hosted: only portal is
  reachable; agent connections always go through it. The
  dispatcher pattern from ADR 0006 + the proxy pattern from this
  ADR compose cleanly across topologies.
- **New external integrations attach to the existing edge.**
  Third-party agents, alternative agent runtimes, future remote
  workers can attach to portal's existing authentication /
  authorization model rather than discovering a separate auth
  surface on stiglab.

### Negative / trade-offs

- **Portal carries WebSocket lifecycle complexity.** Two tokio
  tasks per connection, frame forwarding, heartbeat passthrough,
  graceful close on either side. This is real new code in portal —
  axum's WebSocket support plus `tokio-tungstenite` as a backend
  client are well-trodden paths, but it is not zero work.
- **One extra hop (loopback) per agent message.** Negligible
  latency in single-container deployments. Could matter if
  multi-container topology is adopted later — but at that point
  the loopback becomes inter-container DNS, which is the same
  shape Caddy already does for the dispatcher in dev.
- **Authentication model needs alignment.** Today's WebSocket auth
  is whatever stiglab does (or doesn't); portal's auth is
  PAT/session. v1 keeps the current model so the move is purely
  structural; tightening is a follow-up.

### Neutral

- **Agent CLI's URL is unchanged** (`wss://host/agent/ws`). No
  agent-side rebuild. Existing nodes reconnect without
  configuration change.
- **Stiglab keeps the protocol implementation.** Internal
  agent-runtime concern is unaffected. The `AgentMessage` /
  `ServerMessage` types in `crates/stiglab/src/core/protocol.rs`
  do not move.
- **ADR 0001's runtime invariant is preserved.** Portal is at the
  seam, stiglab handles internal agent-runtime work, no new sync
  HTTP between Onsager subsystems. The portal-to-stiglab loopback
  WebSocket is internal transport, not cross-subsystem RPC.

## Dev-process counterpart

Per ADR 0002, the dev-process analog: `xtask check-api-contract`
(already in CI) gains a stricter check — every external route must
be hosted by portal. No allowlist for sibling-subsystem-hosted
external routes. The check hard-fails on any new `route(...)` /
`get(...)` / `post(...)` registration in
`crates/{stiglab,synodic,forge,ising}/src/server/` that is not
loopback-bound or otherwise gated as internal.

This makes the "no subsystem carve-outs" rule mechanical, not
review-time discretion. The same posture ADR 0004 Lever B
established for cross-subsystem HTTP is now extended to clause 1.

## Adoption checklist

Implementation lives in the spec opened alongside this ADR. Status
as of 2026-05-09:

- [ ] Add `crates/onsager-portal/src/handlers/agent_ws.rs` —
      WebSocket upgrade handler that authenticates the connection
      and opens a loopback WebSocket to stiglab.
- [ ] Add `tokio-tungstenite` (or equivalent) to portal's
      `Cargo.toml` for the backend WebSocket client.
- [ ] Wire `/agent/ws` route in `crates/onsager-portal/src/
      server.rs`.
- [ ] Stiglab: rename internal route to `/agent/ws-internal` and
      bind only to `127.0.0.1`.
- [ ] Update dev Caddyfile (`deploy/dev/Caddyfile`): `/agent/ws`
      now routes to `portal:3002`.
- [ ] Update production Caddyfile (from #283's implementation):
      `/agent/ws` now routes to `127.0.0.1:3002`.
- [ ] Tighten `xtask check-api-contract`: any external route
      registered outside portal is a hard failure (no allowlist).
- [ ] Add an "Amendment 2026-05-09" section to ADR 0006 noting
      that the stiglab carve-out was closed by ADR 0008.
- [ ] Update root `CLAUDE.md` with the closed carve-out.
- [ ] Flip ADR 0008 to `Accepted` in the same PR that lands the
      implementation.

## Out of scope

- **Replacing the WebSocket with HTTP polling** (Option 2 from the
  2026-05-09 conversation). Filed as a possible follow-up if the
  streaming model proves to be wrong long-term.
- **Spine-as-truth for every agent message** (Option 3 from the
  conversation). Same — possible follow-up.
- **Tightening agent authentication** (PAT/session instead of
  whatever's there today). Filed as a follow-up; v1 keeps the
  current auth model so the move is purely structural.
- **Multi-tenant agent dispatch** (per-workspace node pools).
  Existing concern, unaffected by this ADR.
- **Migrating the protocol from WebSocket to HTTP/2 streams or
  gRPC.** Wire format unchanged; only the route owner changes.
