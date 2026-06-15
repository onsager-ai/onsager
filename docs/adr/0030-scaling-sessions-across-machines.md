# ADR 0030 — Scaling agent sessions across machines

- **Status**: Accepted (owner go 2026-06-15 — building; supersedes the
  original "direction-only / not pre-launch" stance, see § Build decision)
- **Date**: 2026-06-15 (proposed as direction); 2026-06-15 (accepted to build)
- **Identity impact**: yes — amends commitment 1's "one factory process / no
  second process": sessions may run on dialed-in worker machines. The
  load-bearing half (Postgres is the spine; durable truth never leaves it)
  is kept — workers hold no database. See § Identity rationale.
- **Amends**: [ADR 0029](0029-one-process-progressive-rebuild.md)'s
  one-process commitment (was: scaling deferred "until a second machine is
  real"). The owner decision (2026-06-15) is to build now, using this dev
  machine as the first worker — arbor's "local machine = default machine"
  pattern.
- **Tracking issues**: see § Milestone ladder
- **Supersedes**: none
- **Superseded by**: none
- **Sibling reference**: arbor [ADR 0006](https://github.com/onsager-ai/arbor/blob/main/docs/adr/0006-scaling-build-jobs.md)
  (scaling build jobs across the fleet), whose builder-machine model and
  no-scheduler guardrail this ADR follows. arbor's 0004/0005 (docker app
  hosting, http tunnel) do **not** apply — Onsager has no host role.

## Context

Onsager's whole workload is the agent session: `scheduler::fire` inserts a
`runs` row and `tokio::spawn`s a supervised task that runs the `claude` CLI
in-process (`sessions::run_agent_session`), which both drives the CLI and
writes `session_events` to Postgres. There is today **no concurrency cap** —
every fire spawns unbounded.

The session is the scaling bottleneck, and it hits two independent ceilings
as load grows (arbor 0006 Context, same shape):

1. **Compute.** A `claude` session is CPU/RAM/disk heavy; one process runs
   only a handful concurrently and is a single point of failure.
2. **Anthropic quota.** Sessions sharing one `CLAUDE_CODE_OAUTH_TOKEN` share
   one subscription's rate limits. More compute does not raise this ceiling.

ADR 0029 accepted this and deferred horizontal scaling "until a second
machine is real." This ADR records the **planned exit** — what the split
looks like and where its seams are — so the bottleneck has a named direction,
not a future rewrite. The MVP stance (below) keeps today's model.

## Decision

### D1 — The control plane becomes a coordinator; the session runs on a machine

When the split is taken, the control plane stops running sessions locally and
becomes a **scheduler/coordinator/aggregator**: it persists a session job
`{run_id, session_id, stage, credential_ref}`, dispatches it to a machine,
and aggregates the machine's event stream into the dashboard SSE. Capacity =
Σ machine slots; scaling = adding machines. (arbor 0006 D1.)

### D2 — A machine is a "builder", not a dumb exec

Onsager's machine maps to arbor's **builder** role, not its dumb docker-exec
host (arbor 0005 D2 / 0006 D2). The machine runs the whole `sessions.rs`
pipeline locally — spawn `claude`, read stdout NDJSON, normalize to
`session_events` — and streams **normalized run events** back over its
outbound channel, not raw stdout. The control plane keeps job construction
(env, `--mcp-config`, credential injection) and persistence; the machine
keeps execution. Onsager has **no host role**, so arbor 0006 D6's
builder/host split is N/A — Onsager is strictly one capability.

### D3 — Auth scales per-identity — already satisfied

arbor 0006 D3 has to *introduce* per-user credentials because one shared
token is a global throttle. Onsager already stores credentials per workspace
and injects them into the session env. So the Anthropic-quota ceiling already
scales with workspaces by construction; the dispatch frame carries the job's
`credential_ref`, no new credential model required.

### D4 — Leased run state — at-most-once, NOT arbor's re-queue

A dispatched job is **leased** to a machine that heartbeats progress; a crash
or disconnect expires the lease. This is the one place Onsager **cannot copy
arbor**. arbor re-queues an expired build because a build is idempotent (it
produces a versioned artifact). **Onsager sessions are not idempotent** — a
session pushes branches, opens PRs, and comments on issues; blind
re-dispatch would double-act. Onsager's lease semantics are therefore
**at-most-once**: lease expiry **fails the run** (surfaced loudly), it does
not re-queue. Automatic retry of a partially-executed session is explicitly
rejected.

### D5 — Placement stays dumb until it can't (ADR 0029 guardrail holds)

Round-robin / least-loaded over connected machines. No hand-rolled
bin-packing, autoscaling, or failover: the moment placement needs to be
smart, it becomes a **driver** over a real scheduler (k8s / Nomad / Fly)
behind the same seam. We never grow our own scheduler. (arbor 0004 D5 /
0006 D5.)

### D6 — Transport: machines dial out (when built)

A machine connects **outbound** to the control plane's one public endpoint
over a persistent WebSocket, authenticates with a control-plane-minted
revocable token, registers `{name, labels}`, and subscribes to work. Nothing
inbound; NAT/VPN out of the required infrastructure. The local machine (=
the control plane's own host) is the **default machine**, not a special case
— it runs the same dispatch interface in-process. (arbor 0005 D1/D3, 0006.)

## The named seams

So the split is a seam swap, not a rewrite, three places carry it:

- **`scheduler::fire`** — `tokio::spawn(run_agent_session)` becomes
  `dispatch(job) → machine`. The local machine is the degenerate dispatch
  target (in-process), keeping every current flow byte-compatible.
- **`sessions::run_agent_session`** — the CLI-drive + NDJSON-parse +
  `session_events` write is the *builder pipeline*; it moves to the machine
  wholesale. Today it runs on the control plane because the control plane is
  the only machine.
- **Event flow inverts** — `session_events` are produced on the machine and
  forwarded to the control plane to persist + `pg_notify` the dashboard,
  instead of written in-process.

## Build decision (2026-06-15) — supersedes the original MVP stance

The ADR first landed (#644) as *direction-only*: build nothing but the
concurrency cap, defer the fleet until "a real second machine is carrying
real load," because pre-launch the fleet has no consumer (commitment 4). The
owner decision on 2026-06-15 **overrides that**: build the fleet now, with
**this dev machine as the first worker** — arbor's "local machine = default
machine." The consumer is real (a second execution host on this box), so
commitment 4 is satisfied; the override is deliberate and recorded here, not
silent.

### Milestone ladder

- **M1 — concurrency cap.** Bounded `scheduler::fire` semaphore. *(Done, #644.)*
- **M2 — transport + dispatch (happy path).** *(Done, #646.)*
  `/api/fleet/connect` (dial-out WebSocket, machine-token, fail closed);
  `onsager worker` subcommand; `dispatch()` routes a session to a connected
  machine (else in-process); event-flow inversion (worker forwards
  `Event`/`Delta` frames, control plane persists + fans out); abort routed to
  the owning machine.
- **M3 — at-most-once lease + reclaim.** *(Done.)* Worker heartbeat +
  control-plane lease reaper (silent machine → its sessions fail); worker
  reconnect with capped backoff; boot-time crash reclaim
  (`scheduler::reclaim_orphaned_runs` fails runs/sessions a dead predecessor
  left `running`). A disconnect/crash **fails** the run (never re-queue —
  sessions aren't idempotent, D4). Registry stays in-memory; durable
  machine-registry persistence folds into M4.
- **M4 — dashboard fleet view.** *(Done.)* A `machines` table (migration 005;
  the 12th table) upserted online on connect / offline on disconnect+reap; a
  `GET /api/fleet/machines` operator endpoint overlaying live in-flight counts;
  a **Machines** dashboard page. **Nav-noun decision:** Machines is an
  *operator/infrastructure* surface, sitting alongside the existing Activity
  and Credentials operator nav items — **not** a fifth user-facing product
  noun. The 4-noun product vocabulary (Workflow/Run/Artifact/Stage) is
  unchanged, so M4 needs no separate identity ADR.

Per-workspace credentials (D3) already satisfy the quota axis and need no
milestone.

## Identity rationale (required by the identity-impact flag)

- **Commitment 1 ("one process; coordination is function calls; no second
  process") is amended.** Sessions may now run on a worker machine that dials
  in over a WebSocket — a second process and a coordination protocol (frames),
  exactly what 0029 removed. This is the weighed cost (see § Rejected
  alternatives), taken because horizontal capacity now has a consumer. The
  **load-bearing half is kept**: Postgres stays the single source of durable
  truth and *workers hold no database* — every durable write (artifacts via
  MCP, `session_events` forwarded for the control plane to persist) lands on
  the spine. The re-split runs along the seam 0029 preserved.
- **Commitment 4 ("no producer without a consumer")** is honored: the fleet
  ships with its consumer (a worker on this machine actually running
  sessions), not speculatively. M3/M4 producers (lease columns, a machines
  table) land with their readers in their own PRs.

## Rejected alternatives

- **Postgres `SELECT … FOR UPDATE SKIP LOCKED` work-claim** (identical
  replicas pull jobs from `runs`, no dial-out protocol). Simpler and more
  literally faithful to 0029's "Postgres is the seam"; rejected because it
  requires every worker to reach Postgres directly (untenable for BYO /
  NAT'd machines) and offers no path to the "turn any machine into a worker
  with one command" onboarding story. The dial-out model is chosen
  deliberately, with this protocol cost accepted.
- **Copy arbor 0006 D4 re-queue** — rejected; Onsager sessions are not
  idempotent (see D4).
- **Build the fleet now, pre-launch** — first rejected (no consumer), then
  accepted by owner decision 2026-06-15 once the dev box is used as a real
  worker (§ Build decision). The consumer is the worker; commitment 4 holds.

## Dev-process counterpart (per ADR 0002)

The same discipline this ADR applies to code applies to the work: a scaling
bottleneck gets a *recorded direction with named seams* before it gets built,
so the change is a seam swap reviewers can check against this ADR — not a
surprise rearchitecture justified after the fact. When the owner overrode the
defer, the override was written into the ADR (§ Build decision) before the
code merged — the claim-honest move is to amend the spec first, never to ship
under stale wording.

## Adoption checklist

- [x] Decision recorded; index + CLAUDE.md updated. (#644 for direction)
- [x] M1 — bounded concurrency cap on `scheduler::fire`. (#644)
- [x] M2 — transport + dispatch + event inversion + abort routing. (#646)
- [x] M3 — at-most-once lease + reclaim (heartbeat, reaper, reconnect, boot
      reclaim). (#648)
- [x] M4 — `machines` table + `GET /api/fleet/machines` + Machines page
      (operator surface, not a product noun) — this PR.

## Out of scope

- The host role, app hosting, and the HTTP tunnel (arbor 0004/0005) —
  Onsager has none.
- Capacity-aware scheduling / autoscaling / failover (D5 — driver over a
  real scheduler if ever needed).
- Build/host tier separation (arbor 0006 D6) — N/A, no host role.
