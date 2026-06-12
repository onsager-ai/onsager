# ADR 0029 — 0.5 reset: one factory process, events as record, progressive rebuild from a legacy baseline

- **Status**: Proposed (awaiting go/no-go on #620)
- **Date**: 2026-06-12
- **Identity impact**: yes — amends commitment 1 (event-bus coordination
  model) and narrows the enforcement surface of commitment 4; see
  § Identity rationale.
- **Adoption**: not started
- **Tracking issues**: #620 (umbrella; milestone sub-specs listed there)
- **Supersedes**: ADR 0001's coordination model (events as inter-process
  transport), ADR 0027's two-process topology, ADR 0004/0018's
  enforcement surfaces (lints return per need). ADR 0010 (provenance),
  0012 (executor catalog), 0017 (plan compilation) carry forward as
  concepts; their v1 implementations move to `legacy/` and port back
  by justification.
- **Superseded by**: none

## Context

Two waves of consolidation (ADR 0027, 0028) deleted the dormant half of
the original architecture and unified its duplicated live structure.
What remains is sound but still shaped like the system it was designed
to become, not the system it is:

1. **The event bus has ~4 real edges.** Engine consumes one event kind
   (`plan.run_requested`); portal consumes three. 18 of 30 manifest
   kinds are diagnostic-only. A bus with four edges is a function call
   with extra steps — and every new feature pays the
   emit/manifest/listener/decode toll before it can do anything.
2. **The two processes are co-located everywhere.** One container in
   production (`deploy/onsager.Dockerfile`), one VM in dev. A whole
   complexity class exists only to serve the process boundary: the
   plan-token encrypt/embed/decrypt hop (#536), encrypted repo tokens
   in event payloads, listener plumbing on both sides, `check-events`,
   the re-scoped `lint-seams`.
3. **Two agent-session runners do the same job** — portal's in-process
   `session_runner` (chat) and the engine's `runners` (workflow runs),
   both spawning the claude CLI via `onsager-agent-spawn`.
4. **Debugging is cross-process by construction.** The 0.4 wave-close
   e2e needed three service restarts, two log files, and watermark
   queries against `events_ext` to trace one prompt string across an
   executor serde boundary, a singleton-config hop, and a process
   boundary (#616).
5. Beyond the topology, several concepts carry no current consumers:
   the spec-plan layer and its fifth nav noun, Script/Verify executors
   (unregistered at runtime), the five-invariant validator guarding a
   one-value provenance system, workflow versioning with no reader,
   a Cron trigger kind nothing fires.

The 0.4 lesson generalizes: transforming this in place means every PR
fights the existing structure and its enforcement apparatus. The owner
decision (2026-06-12) is to **reset**: move the current tree to
`legacy/`, start a fresh minimal workspace, and rebuild progressively —
porting from `legacy/` only what a milestone justifies.

## Decision

1. **One factory process.** A single binary owns the HTTP edge
   (dashboard API, webhooks, OAuth, credentials, MCP server) and the
   scheduler (a supervised tokio task). Run requests are function
   calls. Session tokens are arguments, not encrypted payloads. One
   session-spawn path serves chat and workflow runs.
2. **Events are the record, not the protocol.** Postgres remains the
   single source of durable truth: state tables plus an append-only
   events table written for audit, the dashboard activity feed, and
   crash recovery. `pg_notify` survives only to push live updates to
   the dashboard. No listener dispatch, no manifest discipline, no
   event-RPC. Stigmergy returns as a deliberate choice when a second
   real consumer (or second machine) exists — the same
   retire-until-real rule ADR 0027 applied to observers.
3. **Progressive rebuild from a legacy baseline ("zero-based
   porting").** `crates/`, `apps/`, `xtask/`, `deploy/` move to
   `legacy/` (excluded from the workspace, pnpm, and CI — grep-able
   and copy-able, never built). Nothing survives by default; what a
   milestone justifies is **ported, not re-typed** — battle-tested
   modules (GitHub App/OAuth/webhook verify, credential crypto, agent
   CLI spawn, dashboard pages) move wholesale with their tests.
4. **Same repo, same governance.** Issues, ADR lineage, and the
   dev-process (specs as ground truth, claim-honesty, worktree PRs,
   squash merges) continue unbroken. Enforcement *tooling* (xtask
   lints, file budget) is rebuilt per need once there is drift worth
   catching, not ported up front.

## Identity rationale (required by the identity-impact flag)

- **Commitment 1 (event-bus factory) is amended, not discarded.** The
  load-bearing half — *coordination through shared durable state
  rather than fragile in-memory coupling* — survives: Postgres stays
  the spine, the record of every run is replayable, and the dashboard
  reads the same medium the factory writes. What is dropped is the
  claim that *processes* are the unit of isolation and events the
  inter-process protocol. At the current scale (one human + agents,
  one VM, four edges) that claim costs more than it isolates. The
  re-split path stays open precisely because durable state never
  leaves Postgres.
- **Commitment 2 (artifacts + provenance) carries forward.** Artifacts
  remain the unit of meaning and carry `Provenance` from day one of
  the rebuild (it is a cheap tagged enum). The five-invariant
  compile-time validator and the Verify executor return when a
  consumer exists; until then provenance is data, not machinery.
- **Commitment 3 (specs are ground truth) is untouched** and governs
  the rebuild itself: every milestone is a spec with a Test section,
  and `legacy/` deletions are recorded, not silent.
- **Commitment 4 (internal symmetry) is untouched as a value**; its
  lint surface is rebuilt per need.

## The milestone ladder

Each milestone is an umbrella sub-spec, lands as ordinary PRs, and
gates on an e2e the previous milestone could not pass. Anything not
named here stays in `legacy/` until a milestone justifies it.

- **M0 — skeleton.** Fresh workspace: one crate (`onsager`), axum +
  sqlx + fresh migration set, dev-login, health; the dashboard shell
  (ported app frame) boots against it; CI is fmt + clippy + test.
- **M1 — the loop.** Builder-created workflow (manual trigger, agent
  stage) → in-process run → live claude session → **artifact row** +
  run timeline. Ports: `onsager-agent-spawn`, credential crypto, the
  unified session runner, builder + runs dashboard pages. This
  re-proves the 0.4 wave-close e2e on v2 and fixes #619 by
  construction (run products land `artifacts` rows).
- **M2 — GitHub.** App auth, webhook receiver, issue-labeled trigger,
  read-scoped repo access, PR-opening flow. Ports `onsager-github`
  wholesale.
- **M3 — operate.** Runs list/detail, activity feed (reading the audit
  events), abort, credentials UI, artifact detail.
- **M4 — flip.** MCP server + chat surface re-judged and ported if
  still wanted (owner decision at the gate); `legacy/` deleted (git
  history keeps it); root CLAUDE.md identity section rewritten to
  match this ADR; open issues re-triaged against v2.

## Consequences

- Dev loop: one service + db + vite; one log; one restart; e2e in one
  process. The devproxy/worktree flow is unchanged.
- The ~56k-line tree drops to whatever the ladder justifies; the prior
  waves suggest the steady state is well under half of that.
- Risk accepted: fault isolation between edge and scheduler moves from
  process boundary to task supervision (agent work stays in
  subprocesses). Independent scaling is deferred until a second
  machine is real (see multi-machine direction note).
- Risk to manage: second-system effect. The ladder's e2e gates and the
  port-don't-retype rule are the mitigations; M1 deliberately rebuilds
  the *proven* loop before anything new.

## Adoption checklist

- [ ] Decision recorded; index updated; go/no-go on #620. (this PR)
- [ ] M0 skeleton landed (legacy move + fresh workspace + CI + shell).
- [ ] M1 loop landed (e2e: builder workflow → run → artifact row,
      one process, zero seeding).
- [ ] M2 GitHub landed (e2e: issue label → run → PR).
- [ ] M3 operate landed (runs/activity/credentials surfaces).
- [ ] M4 flip: MCP/chat decision recorded; `legacy/` deleted;
      CLAUDE.md identity rewritten; open issues re-triaged; flip
      `Adoption` to `enforced`.
