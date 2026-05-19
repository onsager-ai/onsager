# Onsager

AI factory stack — monorepo for the Onsager event bus and its subsystems.

## What makes Onsager Onsager

These commitments define Onsager-the-factory's identity — the monorepo and
its subsystems. They do not, in general, prescribe the internal structure
of what the factory produces; downstream artifacts and deployed systems
are their own viable systems with their own identities.

The exception is **how** the factory operates on its work (specs as ground
truth, below) — that commitment binds both Onsager's self-construction and
its production discipline.

- **Event-bus factory, not service mesh.** Subsystems coordinate through
  the spine (events, pg_notify, shared tables), not through synchronous
  calls. [ADR 0001](docs/adr/0001-event-bus-coordination-model.md) is
  this commitment's first concretization. Scope: factory only.
- **Artifacts are the unit of meaning.** Every persistent, lifecycle-bearing
  object in the factory is an artifact — internal-authored ones (specs,
  designs) and external-referenced ones (PRs, Issues) alike. Factory
  subsystems (Forge, Stiglab, Synodic, Ising) operate on artifacts; events
  are state-change notifications, not first-class entities. Scope: factory
  only — products of the factory have their own ontology.
- **Specs are ground truth, code is downstream.** When spec and code
  conflict, the code is wrong. Specs are amended deliberately; code is
  amended to match. Scope: both factory self-construction and factory
  production discipline — Onsager sessions producing artifacts inherit
  this constraint.
- **Internal symmetry is load-bearing.** Equivalent concepts must have
  equivalent shapes (names, types, error models, write paths). Asymmetry
  between equivalents is a defect, not a polish concern. The seam rule
  and three lints (`lint-seams`, `check-events`, `check-api-contract`)
  are this commitment's enforcement surface. Scope: factory only;
  applies to session-produced code when it lands in the monorepo.

Changes to the four bullets above carry an `Identity impact: yes` flag
in the modifying ADR and require explicit rationale. Changes to anything
below in this file are by default `Identity impact: no`. ADRs themselves
may carry either value — the flag tracks whether the ADR touches the
four bullets above, not where the change lives.

### Honoring commitment 3: claim-honesty checks

Commitment 3 binds both directions. Code drifting from spec is a bug;
shipping under a Plan item without having met it is the same bug from
the other side. Before claiming a Plan item done — in a session message,
a checkbox tick, or a PR description — pass these checks. The point is
not to inflate scope; if a check exposes that the spec was wrong, amend
the spec, then ship.

- **"The main path works, so it's essentially done."** Plan items are
  atomic. There is no "essentially." Either the item is complete, or
  split it into `N.1` (done) and `N.2` (deferred, with reason) and
  amend the spec before merge. "Essentially done" is narrative, not
  state.
- **"Tests pass, so the change is correct."** A green CI proves the
  test suite did not catch a bug, not that your code is exercised.
  Before claiming done, break the new code path locally (mutate one
  line); the corresponding test must fail. Green CI plus uncovered new
  logic is theater.
- **"I'll handle the edge case as a follow-up."** A follow-up that has
  no issue does not exist. Either open a sub-issue (`Part of #N`) and
  narrow the current PR's stated scope, or do it now. Untracked defers
  become permanent corners.
- **"The workaround is fine; the proper fix is too invasive."** The
  judgment may be right. Honesty requires one of: (a) amend the Plan
  item to read "workaround, because X"; or (b) open a follow-up issue
  for the proper fix and link it. Shipping a workaround under
  unchanged Plan wording is silent scope reduction.
- **"`cargo check` passes, so the refactor holds."** Compilation proves
  types align, not that behavior is preserved. Refactors carry the
  same proof obligation as features: tests pass and the changed paths
  are exercised by them. Without that, you are shipping a hypothesis.
- **"To be done, I should test every edge case I can think of."** No.
  Done means the spec's Test section is satisfied. If the Test section
  is too thin or too fat, the spec is wrong — amend it. Silently
  exceeding or shrinking the bar is the same defect as silently
  delivering less than the Plan.

### Named failure modes

The rebuttals above point at recurring shapes. Naming them lets PR
review, spec comments, and session messages reference them in one
token instead of re-describing the failure each time. When you spot
one — in your own work or someone else's — name it.

- **claim ≠ reality** — the umbrella name for this whole annex: a
  session asserts done while reality has not met the spec.
- **silent scope reduction** — shipping a workaround, partial fix, or
  narrowed behavior under a Plan item whose wording still describes
  the full scope. The workaround may be the right call; shipping it
  without amending the Plan or opening a follow-up is the defect.
- **theater coverage** — green CI plus tests that do not actually
  exercise the new code path. Catch it by mutating a line of new code
  locally; if no test fails, the coverage is theater.
- **narrative-as-state** — using prose like "essentially done",
  "basically works", "mostly complete" in place of an atomic Plan
  item state. Plan items are binary; either complete, or split and
  amended.
- **untracked defer** — a follow-up that lives only in a session
  message or PR description, with no issue. It evaporates at
  squash-merge.

These names apply symmetrically: catching them in your own draft
before claiming done is the same skill as flagging them in review.

See [ADR 0005](docs/adr/0005-s5-governance-scales-with-scale.md) for
the meta-rule on how this S5 layer evolves with operational scale.

## Operating posture: pre-launch

Onsager has not yet been launched live to users. That fact removes
one category of scaffolding from our work — the kind that protects
users from our half-finished state. It does NOT loosen the internal
discipline that produces well-built work in the first place. The
point of pre-launch is to ship more learning per unit time, not to
ship less rigorously, and not to defer hard problems past launch.

### What pre-launch removes

User-protection scaffolding only makes sense once humans depend on
us; we skip it now:

- **Feature flags meant solely to hide work from a userbase that
  doesn't exist yet.** When a surface lands, it lands visibly.
  Flags are still legitimate for in-flight A/B work or
  workspace-level admin choices (e.g. "this workspace's owner
  doesn't want the agent enabled") — but not for hiding incomplete
  surfaces from a userbase of zero.
- **Mock implementations as bridges to absent dependencies.** If
  spec A needs spec B and B hasn't landed, reorder the work so B
  lands first. Throwaway mocks become "bridges that ossify"
  (see § "Architectural drift patterns to watch") — pre-launch is
  exactly when you can pay the reorder cost cheaply.
- **"Preview" / "beta" banners** on surfaces that are the product
  for our internal team. We know what we're building; we don't
  need to warn ourselves.
- **Bookmark / deprecation preservation work** for routes nobody
  has bookmarked yet. Delete the old route and its backend handler
  in the same PR as the replacement. Redirects are still cheap and
  worth adding when free; long deprecation windows are not.

### What pre-launch does NOT loosen

Pre-launch is not a license to ship less rigorous work. Internal
discipline matters *more* at high velocity, not less — code we
touch faster needs more guardrails to catch the mistakes that
velocity invites, not fewer:

- **Spec discipline.** Specs remain ground truth (commitment 3).
  Pre-launch lets us amend specs more cheaply, not skip them.
  Implementation without a spec is *untracked defer* (named
  failure mode above) — the absence of users does not change that.
- **Claim-honesty.** "Done" still means the spec's bar is met,
  not "essentially works." *Theater coverage* (green CI with
  uncovered new code) is theater whether we have users or not.
  Plan items are still atomic; split-and-amend, don't silently
  reduce scope.
- **Tests.** "Nobody will hit this path yet" is not a reason to
  skip tests. Tests are how we know the code works *before* we
  find out it doesn't. The claim-honesty annex's mutation check
  (mutate one line of new code; a test must fail) applies
  regardless of launch status.
- **Review.** Velocity comes from small PRs reviewed quickly, not
  large PRs reviewed never. Self-review (the claim-honesty checks
  in commitment 3) is the floor; a second pair of eyes is the
  norm. Squashing a PR onto `main` does not retire the review
  obligation.
- **Root-cause fixes over band-aids.** Pre-launch is the cheapest
  moment to fix a problem properly — no migration choreography,
  no in-flight runs, no operational scar tissue. A workaround
  shipped under unchanged Plan wording is *silent scope
  reduction*. A workaround shipped with a follow-up issue and an
  amended Plan is fine. A workaround shipped with neither is the
  longest-half-life untracked-defer pattern — "I'll fix it after
  launch" almost never holds, because launch makes the fix more
  expensive, not less.
- **Internal symmetry, seam rule, file budget, lint enforcement.**
  `lint-seams`, `check-events`, `check-api-contract`, and
  `check-file-budget` stay hard-fail. Code structure rots *faster*
  pre-launch, not slower, because we touch more of it per unit
  time.
- **Identity commitments** (the four bullets at the top of this
  file). Those define the factory, not its launch status.

### What pre-launch is for

The window only opens once. Use it for moves that are cheap now
and expensive later:

- **Reordering work** when a dependency surfaces — nobody depends
  on the current sequence.
- **Schema changes and migrations** without choreography — no
  production data to preserve, no zero-downtime constraint.
- **Discovering wrong abstractions** and rewriting them — no
  public contract yet, no client code to coordinate with.
- **Replacing whole subsystems** when the original guess was
  wrong — no operational scar tissue, no on-call rotation to
  brief.

Use the window for those *structural* moves. Quality shortcuts
the discipline above forbids are not the same thing — they don't
get cheaper at launch, they get more expensive. **Pre-launch
removes user protection. It does not remove work-quality
protection.**

### Flipping the posture

When we launch, delete this section and replace it with the
post-launch operating bars (bookmark preservation, deprecation
windows, feature-flag gating for high-blast-radius surfaces,
mock-implementation policy for unmerged dependencies,
communication discipline for breaking changes). The flip is
itself a deliberate ADR-worthy moment — landing live to users is
a commitment, not a deploy event.

## Architecture

See § What makes Onsager Onsager (above) for the identity commitments
these architectural choices instantiate.

Onsager is a **factory event bus** architecture. Subsystems are runtime-decoupled
via a shared PostgreSQL `events` / `events_ext` table + `pg_notify` channel.
They coordinate through stigmergy (indirect signals via shared medium), not
direct calls.

See [ADR 0001](docs/adr/0001-event-bus-coordination-model.md) for the
decision and migration checklist.

[ADR 0002](docs/adr/0002-process-product-isomorphism.md) frames design as
two loops — the **inner loop** (spec → PR → merge) and the **outer loop**
(observe drift → propose rule → activate rule → modify inner loop) — and
commits us to process ↔ product isomorphism: every factory primitive
ships with its dev-process counterpart enabled, and every durable
dev-process pattern is filed as evidence for a future primitive.

```
                onsager-spine (event bus lib)
         /     /     |        |        \
   portal  forge  stiglab  synodic   ising
   (edge)         (factory subsystems — AI-native concerns)
```

**Architectural invariant**: subsystems (`portal`, `forge`, `stiglab`,
`synodic`, `ising`) must NOT import each other, and must NOT be statically
linked into the same binary. The `onsager` dispatcher has zero business
dependencies -- it discovers subsystem binaries on PATH.

`portal` is the **edge** subsystem — the only one that hosts public
HTTP routes (dashboard API, GitHub webhooks, OAuth, credential CRUD,
the agent control-plane WebSocket). Factory subsystems (`forge`,
`stiglab`, `synodic`, `ising`) live behind the seam and coordinate
exclusively via the spine. The route-level move landed via spec #222;
the process-level move landed via [ADR 0006](docs/adr/0006-edge-dispatcher-as-the-public-boundary.md)
(spec #283 — Caddy in front of portal in production) and
[ADR 0008](docs/adr/0008-portal-owns-the-agent-control-plane.md)
(spec #291 — portal terminates `/agent/ws` and proxies bytes to
stiglab on loopback). Stiglab no longer accepts external
connections at either layer.

In production the externally-reachable process is Caddy (the edge
dispatcher), bundled in the same image. Stiglab binds to
`127.0.0.1:3000` and serves only `/agent/ws-internal`; portal binds
to `127.0.0.1:3002` and owns every external route, including the
public `/agent/ws` it forwards to stiglab over loopback.

## MCP server + public skills bundle

[ADR 0007](docs/adr/0007-tools-and-skills-as-the-public-contract.md)
names the **protocol shape** of clause 1 for AI runtimes: portal
hosts an **MCP server** at `POST /mcp/messages` (JSON-RPC 2.0 over
HTTP), and a sibling **public skills bundle** at
`onsager-ai/onsager-skills` packages the operating-procedures
knowledge that pairs with the tools (`npx skills add
onsager-ai/onsager-skills`).

The MCP server is portal's clause-1 surface for AI clients (Claude
Code, Cursor, Codex, custom agents, *and* the dashboard chat —
which becomes one MCP client among many). The same workspace-scope
auth (`AuthUser` extractor + `require_workspace_access`) gates
both REST and MCP. Tools delegate to the same DB helpers the REST
handlers use — no new business logic, just typed wrappers.

Tool schemas SSOT: derived from Rust serde structs via `schemars`
(`#[derive(JsonSchema)]`) — no hand-written JSON Schema, no
parallel-source-of-truth drift. The TS-side counterpart (generated
TS from the same Rust structs) is a follow-up.

Layers landing in stages (#288):

- **MCP backend slice** — `crates/onsager-portal/src/mcp/`, 7
  action tools + 4 diagnostic tools (`propose_remediation` is a
  v1 stub returning log pointers; server-side AI reasoning is a
  follow-up), Caddyfile `/mcp/*` block, `xtask
  check-tools-and-skills` lint, ADR 0007 flipped to Accepted.
- **Skills bundle (#310, migrated #323)** — the four initial public
  user-facing skills (`onsager-design-workflow`, `onsager-run-workflow`,
  `onsager-triage-run`, `onsager-explore-artifacts`) live canonically
  in `onsager-ai/onsager-skills`. Install with
  `npx skills add onsager-ai/onsager-skills` or
  `git clone https://github.com/onsager-ai/onsager-skills`. See the
  sibling repo's `README.md` for the trigger-phrase matrix and how the
  skills compose into one product loop. Cross-repo dev-process skills
  (`plan-dag`, `issue-spec`, `ci-triage`, `web-testing`, `railway`,
  …) now live in `onsager-ai/dev-skills` and install globally via
  `npx skills add -g onsager-ai/dev-skills --skill '*' -a claude-code`.
- **Dashboard MCP client + HitlCard primitive (landed, spec #311).**
  `apps/dashboard/src/components/chat/HitlCard.tsx` renders the three
  HITL card shapes (constructive / diff / destructive) over one
  primitive; `apps/dashboard/src/lib/mcp-tools.ts` is the typed view
  over the Rust registry (hand-typed for v1, schemars-to-TS codegen
  is a follow-up). `ChatBuilder.tsx` is now a same-origin MCP client
  that calls the Anthropic SDK with prompt caching (per the
  `claude-api` skill) and routes every mutation tool call through a
  HitlCard, every read-only call through a plain info block.
  `xtask check-hitl-coverage` hard-fails on drift between the Rust
  registry and the dashboard bindings (HITL principle 1, enforced
  mechanically).

`xtask check-tools-and-skills` is the enforcement counterpart of
ADR 0007's dev-process clause (every public tool has a skill
grant; every skill grant references a real tool). It runs in CI
via a transient `git clone` of `onsager-ai/onsager-skills` (spec
#323) and is part of `just lint`. For local two-checkout dev, set
`ONSAGER_SKILLS_DIR=../onsager-skills just lint`; without the
override, `just lint` clones the sibling into `target/onsager-skills/`
automatically.

**Shared skill editing.** Skills installed under `.claude/skills/` via
`npx skills add onsager-ai/onsager-skills` are read-only copies — do not
edit them directly. A `PreToolUse` hook (`.claude/hooks/check-skill-edit.sh`)
blocks direct edits to any skill directory that carries a `.upstream-source`
marker file. To change a shared skill:

1. Edit it in `onsager-ai/onsager-skills`.
2. Open a PR there, get it reviewed, and merge it.
3. Re-run `npx skills add onsager-ai/onsager-skills` in this repo.

Editing the installed copy is blocked by the hook and wrong — a future
`npx skills add` run would silently overwrite the change.

## The seam rule (canonical)

> HTTP APIs exist only at external boundaries:
> - **User-facing endpoints** called by the dashboard.
> - **Webhooks** called by external services (GitHub, etc.).
>
> The external HTTP boundary is owned by `portal` (the edge subsystem).
> Factory subsystems (`forge`, `stiglab`, `synodic`, `ising`) coordinate
> **exclusively** via the spine: events on the bus + reads against
> shared spine tables. No subsystem makes HTTP calls to another
> subsystem. No subsystem imports another subsystem's crate.

This is the rule. ADR 0001 set it; [ADR 0004](docs/adr/0004-tighten-the-seams.md)
captures the decision to make it machine-checkable and the six-lever
execution plan that spec #131 tracks (A–F: persisted rule →
mechanical guardrails → finish ADR 0001 migration → spine as SoT →
registry-backed event types → API/UI contract enforcement). All six
levers have landed and are CI-enforced via `lint-seams`,
`check-api-contract`, and `check-events` (see status below); the
seam rule is now mechanical, not review-time discipline.

[ADR 0006](docs/adr/0006-edge-dispatcher-as-the-public-boundary.md)
and [ADR 0008](docs/adr/0008-portal-owns-the-agent-control-plane.md)
close the process-level half of clause 1: production runs Caddy as
the edge dispatcher, portal owns 100% of the external HTTP surface
(including `/agent/ws`), and stiglab is loopback-only. `xtask
check-api-contract` enforces "every stiglab route is loopback-only"
— any new route on a factory subsystem outside the
loopback-only allowlist is a hard failure.

Lever status (canonical: ADR 0004's adoption checklist). As of
2026-04-30: all six levers landed.

- **A** (PR #144): rule persisted in `CLAUDE.md` + skills.
- **B**: `xtask/src/lint_seams.rs` enforces arch-deps, references
  that indicate sibling-subsystem HTTP (sibling `*_URL` / `*_PORT`
  env vars, `localhost:<well-known-port>` literals), `serde(alias)`,
  `*_mirror.rs`, and legacy type aliases. New code must not
  reintroduce references that indicate sibling-subsystem HTTP — the
  lint hard-fails.
- **C** (#148): `HttpStiglabDispatcher` / `HttpSynodicGate` and
  `POST /api/shaping` / `POST /api/gate` are gone. Forge ↔
  stiglab/synodic flows through the spine only —
  `forge.gate_requested` / `synodic.gate_verdict` and
  `forge.shaping_dispatched` / `stiglab.session_completed` (with
  `stiglab.shaping_result_ready` emitted alongside it).
- **D** (#149): `workflow_spine_mirror.rs` is gone, stiglab's
  `workspace_workflows` / `workspace_workflow_stages` collapse into
  the spine `workflows` / `workflow_stages` tables (migration 013
  backfills + drops), the `BundleId` → `ArtifactVersionId` alias is
  gone (#219), and `workspace_install_ref` was renamed `install_id`
  (#219). One schema, one writer.
- **E** (#150, PR #227; tightened by #272): static event manifest at
  `crates/onsager-registry/src/events.rs`, one row per
  `FactoryEventKind` variant, declares producers/consumers per
  subsystem. `xtask check-events` enforces coverage, both-ends-
  declared, emit-call-sites match producers, and listener-call-
  sites match consumers. Per #272, every row is either **real**
  (non-empty `consumers`) or **diagnostic-only** (`diagnostic_only:
  true` plus a non-empty `reason` string identifying what reads it
  today, e.g. dashboard timeline / audit trail). Rows that are
  neither are rejected at lint time. Manifest exposed at
  `GET /api/registry/events`.
- **F** (PR #207): `xtask/src/lint_api_contract.rs` asserts every
  backend route has a dashboard caller (or an allowlisted external-
  only reason) and every dashboard call lands on a backend route.

## Internal aesthetic

Care about the inside the same way you'd care about the outside. The wires
inside an Apple product are routed and dressed even though no user will ever
open the case. We hold the codebase to the same standard: the seams between
subsystems, the shape of internal modules, the consistency of names and
errors, the absence of dead wires — these are first-class quality, not
cleanup chores deferred until "after the feature lands."

This is a value, not a checklist. Three operating principles fall out of it:

- **Internal symmetry is a feature.** When two things are *the same concept*,
  they should have the same shape — same name, same type, same error model,
  same write path. Asymmetry between equivalent things (`TriggerKind` here,
  `TriggerSpec` there; one creation path setting `current_version=0`, another
  setting `=1`) is a defect, even when nothing user-visible is broken.
- **No dangling wires.** Code marked `#[allow(dead_code)]` "for later," event
  types with no consumer, endpoints with no UI caller, and compat aliases with
  no removal date are all the same defect: a wire connected at one end. Either
  finish the connection in the same PR, or remove the loose end.
- **The inside is reviewable.** Files that grow past ~500 LOC, modules that
  mix unrelated concerns, error types that change shape across a subsystem
  boundary — these aren't style preferences, they're a tax on every future
  reader. Splitting and unifying them is real work, worth scheduling.

### File budget

`xtask check-file-budget` (wired into `just lint` and CI) enforces a
**8000 prod-token ceiling** per `.rs` / `.ts` / `.tsx` file. "Prod" means:
test blocks stripped for Rust (`#[cfg(test)]` items); test files skipped
for TypeScript (`*.test.ts`, `__tests__/`).

Tokens are counted with `tiktoken-rs` against the `o200k_base` encoding,
vendored at `xtask/assets/o200k_base.tiktoken` for offline determinism.
Within ~10% of Claude's actual tokenizer; stable across machines.

To **exempt** a file that legitimately exceeds the ceiling (out-of-scope
for an active spec, binary entrypoint, generated file):

```rust
// budget-allow: <non-empty reason explaining why this file is exempt>
```

Place the comment anywhere in the file. Reason text is mandatory and
grep-able. Mirrors the `// seam-allow:` shape.

**Ratchet plan.** The stated value (~500 LOC ≈ ~5000 tokens) is tighter
than the current 8000 ceiling. Once all exempted files are either split or
justified, a follow-up spec tightens the ceiling to 5000–6000 to align
with the stated value. (Spec #261 established 8000 as the initial floor;
ratcheting is a separate spec.)

The seam rule above and the "Architectural drift patterns to watch" list
below are both operational projections of this value onto the seams between
subsystems. Internal-quality work that doesn't fit those projections —
interior-to-a-subsystem hygiene — is equally in scope and should be specced
and shipped on the same footing as feature work.

## User-facing vocabulary (canonical 4 nouns)

Per spec #286 the dashboard, public API field names, route segments,
button copy, page titles, and user-visible docs use exactly four
top-level nouns. Anything else is internal-only or surface-internal
(visible only inside a workflow/run drill-down, never as a top-level
navigation noun).

The four canonical nouns:

- **Workflow** — the automation unit (trigger + ordered stages +
  prompts). Lives at `/api/workflows`. Persisted in spine `workflows`
  / `workflow_stages` (Lever D).
- **Run** — one execution of a workflow against an artifact. Lives at
  `/api/workflows/:id/runs`. Has a status and a sequence of stage
  outcomes.
- **Artifact** — what a run produces (issue, PR, deployment, etc.).
  Already the core noun; lives at `/api/spine/artifacts`. Persisted
  in spine `artifacts`.
- **Stage** — a step within a workflow definition (gate kind +
  parameters). A workflow's structural unit; never a top-level
  navigation noun.

**Demoted to internal-only.** These terms stay rich in Rust /
migration / spine vocabulary but never surface to users:

- **shaping** — legacy term for agent-session dispatch. Stays in
  internal Rust (`shaping_listener.rs`, `ShapingRequest`,
  `ShapingResult`). The user-facing event-name leakage
  (`stiglab.shaping_result_ready`) was renamed to
  `stiglab.session_result_ready` per spec #285.
- **bundle / sealed / ArtifactVersionId** — internal storage terms.
  The user-facing concept is "artifact version".
- **spec** — dev-process term for a GitHub issue with implementation
  intent. Lives in CLAUDE.md and the `issue-spec` skill; never
  surfaces in the dashboard.

**Demoted to surface-internal.** Visible only inside a workflow / run
drill-down, not as a top-level navigation item:

- **gate / verdict** — control points within a stage. Visible in
  workflow detail and run history; never a top-level surface.
- **governance** — the audit/escalation surface. Subsumed into run
  history's verdict view.
- **session** — a stage execution context. Visible only as a stage
  gate kind ("agent-session") and as a drill-down from a run's stage
  history.
- **node** — infrastructure; visible only in settings, not as a
  top-level noun.
- **issue** — the GitHub issue that triggered or was produced by a
  run. An artifact kind, not a separate concept.

**Enforcement is doc-only.** Dashboard tsx is too varied for a
useful grep-based vocabulary lint, and the 2026-05-09 audit found
no significant leakage. The doc commitment plus PR review is the
enforcement mechanism; a mechanical lint can be added later if drift
recurs. This vocabulary is for surfaces users see — the seam-level /
internal vocabulary stays rich per "Internal aesthetic" above.

## Architectural drift patterns to watch

Loose runtime coupling is correct and stays — but the seams it creates are
informal, and recent PRs show drift accumulating in predictable shapes. When
designing or reviewing a change, watch for these and prefer **unification at
the seam** over a bridge. Bridges aren't a destination — collapse them in
the same PR that introduces them.

- **Parallel schemas across subsystems.** If two subsystems each persist their
  own version of the same concept (former drift, retired by Lever D #149:
  stiglab `workspace_workflows` vs spine `workflows`), the spine wins — the
  private table is collapsed into the spine table with a `workspace_id`
  discriminator. The mirror/translator pattern is a bridge, not a destination.
- **Producer with no consumer.** A subsystem can emit events that nothing
  consumes if a consumer is coded but undeployed (PR #127). Treat new event
  types as a contract: producer + consumer + deploy manifest land together,
  or the producer waits.
- **In-memory caches drifting from the bus.** If a subsystem caches state
  that the spine owns, it will drift the moment something changes that state
  out-of-band (PR #123). Default to reading from the spine; only cache with
  an explicit invalidation path tied to a spine event.
- **Half-wired API/UI contracts.** Endpoint shipped without a UI caller, or
  client method shipped without a backend handler (PR #108). Backend and
  dashboard changes for the same surface should land in one PR (or two PRs
  with a contract test that fails until both sides exist).
- **Divergent state shapes from multiple write paths.** If a row can be
  created via two paths (e.g. OAuth callback vs. manual install, PR #122),
  both paths must produce the same shape — or the read side has to be
  defensive in a single, named place, not at every call site.
- **Compat aliases that ossify.** Renames with `serde(alias=...)` or type
  aliases "for one release" (PR #107 `BundleId` → `ArtifactVersionId`) tend
  to outlive their intended window. Land the rename and the alias removal
  in the same PR; don't open a removal-date window. `lint-seams` hard-fails
  on new `serde(alias)` and on legacy type aliases.
- **Denormalized external state.** When an external system (GitHub, Linear,
  Slack, …) is the author of a field, copying it into the spine creates a
  drift surface — every retitle, label edit, author transfer, body rewrite
  is either a webhook we have to chase or a row that quietly goes stale.
  Per spec #170, external-origin artifacts are **reference-only**: the
  spine row carries identity (`external_ref`), *our* derived lifecycle
  (`state`, `current_version`, `last_observed_at`), and *our* relationships
  (lineage, governance verdicts, ising signals) — nothing the external
  system owns. Provider-authored fields (title, body, labels, author)
  are hydrated live by the dashboard through a portal proxy
  (`crates/onsager-portal/src/handlers/live_data.rs`) backed by a short-
  TTL process-local cache (`proxy_cache.rs`, keyed by project/resource,
  shared across all installations in a replica). New external integrations
  inherit this by default: ship a proxy, not a denormalizer. Enforced
  today by review and `crates/onsager-portal/tests/reference_only_artifacts.rs`,
  which pins the existing PR/issue helpers; a mechanical lint for new
  external-origin write paths is a follow-up.

The strategy spec #131 captures the full reasoning and the six-lever plan
that made these contracts enforced. The first five bullets above are now
caught mechanically by `lint-seams`, `check-api-contract`, and
`check-events`; the bullets stay as a glossary of the failure modes those
checks were designed against. The denormalized-external-state bullet is
the exception — review + contract test today, mechanical check pending.

## Workspace layout

```
crates/
  onsager-artifact/    <- domain value objects (Artifact, ArtifactId, BundleId, Kind, lineage, quality)
  onsager-spine/       <- event bus client (EventStore, Listener, Namespace, FactoryEvent)
  onsager-warehouse/   <- bundle sealing + Warehouse trait (depends on artifact)
  onsager-delivery/    <- consumer routing (depends on artifact, warehouse)
  onsager-registry/    <- type registry, seed catalog, evaluators (depends on artifact, spine)
  onsager/             <- dispatcher CLI (~100 LOC, no business deps)
  forge/               <- production line — drives artifacts through their lifecycle (lib + bin)
  ising/               <- continuous improvement engine — observes and surfaces insights (lib + bin)
  stiglab/             <- distributed AI agent session orchestration (lib + bin)
  synodic/             <- AI agent governance (lib + bin)
apps/
  dashboard/           <- React UI (sessions, nodes, governance, factory views)
```

Subsystem → support-crate dependencies (as of #33):

- `forge`   → `onsager-{artifact, warehouse, spine}` (spine carries the request/response DTOs since #131 Lever C; `protocol` is no longer a separate crate)
- `stiglab` → `onsager-{artifact, spine}`
- `synodic` → `onsager-{artifact, spine}`
- `ising`   → `onsager-{artifact, spine}` (no warehouse/delivery/registry)

## Getting Started

Prerequisites: Docker, Rust toolchain (via rustup), pnpm.

```bash
cp .env.example .env            # configure environment (reference for docker-compose)
just dev                        # start Postgres, run migrations, and launch services
just smoke-test                 # verify everything works (in another terminal)
```

To run agent sessions, add your `CLAUDE_CODE_OAUTH_TOKEN` via
Dashboard > Settings > Credentials (encrypted at rest, passed to agents as env vars).

Services:
- **Dashboard**: http://localhost:5173 (Vite dev server with HMR)
- **Stiglab API**: http://localhost:3000 (sessions, nodes, WebSocket)
- **Synodic API**: http://localhost:3001 (governance)
- **Postgres**: localhost:5432 (event spine)

To stop: `Ctrl+C` for services, `just dev-down` for Postgres.

### Parallel dev environments (per-worktree slots)

When two agents (or a human + an agent) need to run the stack on the
same VM at the same time, the **slot system** (#194) gives each
worktree a private, fully-containerized copy of the stack on a
disjoint port block. Slot 0 is the main checkout and uses today's
port layout via `just dev`; slots 1..=99 use a 10-port stride
starting at 9000.

```bash
just worktree-new feat-a              # branch + slot + compose project, all up
just worktree-list                    # see slots, ports, container status
just slot-exec  feat-a cargo test -p stiglab    # one-off command in the slot
just worktree-tunnel feat-a           # SSH `-L` flags for laptop access
just worktree-up    feat-a            # bring an existing slot back after reboot
just worktree-rm    feat-a            # tear down + remove worktree (keeps branch)
just worktree-rm    feat-a --with-branch  # also delete the branch
```

The slot's edge port serves the dashboard and reverse-proxies the
backend APIs same-origin (`/api/synodic/...`, `/api/forge/...`,
`/api/...` → stiglab), so the dashboard makes relative-path fetches
with no per-environment URL config and no CORS surface. SSH-forward
`localhost:9010` (slot 1's edge), open `http://localhost:9010/`,
done. See [spec #194](https://github.com/onsager-ai/onsager/issues/194).

## Build & Test

```bash
just build           # Rust workspace + dashboard
just test            # All tests
just test-all        # All tests including spine integration tests
just lint            # fmt + clippy + eslint
```

Or directly:

```bash
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

## Conventions

- Rust edition 2024, rustfmt formatting, clippy with warnings-as-errors
- thiserror for library errors, anyhow for application errors
- Small focused commits, imperative mood, under 72 characters
- Unit tests co-located in `#[cfg(test)]` modules
- All internal deps use `path = "../..."` -- no git deps, no crates.io

## Merge policy

- **PR → main: squash only.** Merge commits and rebase-merges are
  disabled at the repo level; the GitHub merge button only offers
  squash. One PR = one commit on `main`.
- **main → PR: rebase, not merge.** When updating a PR branch with
  `main` (locally or via the "Update branch" button), use rebase.
  This keeps PR history linear and avoids merge commits inside
  feature branches that would then be squashed away anyway.
- Local equivalent: `git pull --rebase origin main` (or set
  `git config --global pull.rebase true` once).

## Environment variables

Subsystem-specific env vars worth calling out:

- `SYNODIC_FAIL_POLICY` (forge, default `escalate`) — what verdict the Forge
  side returns when the Synodic gate is unreachable, returns 5xx, or its
  response cannot be parsed. One of `escalate` | `deny` | `allow`.
  `escalate` parks the decision non-blockingly (forge invariant #5);
  `deny` keeps the artifact in its current state; `allow` is the legacy
  fail-open behavior and must be opted into explicitly. 4xx responses and
  parse errors always deny regardless of policy — those are protocol bugs
  that should surface loudly.

## File editing (Claude Code tools)

Prefer the `Edit` tool over `Write` for any change to an existing file. Full
rewrites with `Write` can hit a stream idle timeout on files larger than ~150
lines and there is no automatic retry — a stalled `Write` silently leaves the
file in its previous state or, worse, half-written. If a rewrite is genuinely
necessary, split it: write a smaller initial version, then extend with
follow-up `Edit` calls.

## Session defaults (Claude Code cloud)

If the current branch name starts with `claude/` (the prefix cloud sessions
create), treat PR creation and CI auto-fix as part of finishing the task —
do not wait to be asked:

1. Push the branch.
2. Open a pull request. **Before calling `mcp__github__create_pull_request`,
   answer the spec-vs-trivial gate** (the same gate `pr-spec-sync.yml`
   enforces) and bake the answer into the PR at creation time:
   - If a spec issue exists or you should write one, include `Closes #N` or
     `Part of #N` in the PR body.
   - If the change is genuinely `trivial` (typo, doc-only, formatting,
     one-line obvious fix — see `onsager-dev-process` for the full list),
     pass `labels: ["trivial"]` on creation.
   - Default is spec, not trivial. When in doubt, create the spec issue
     first via the `issue-spec` skill, then open the PR with `Closes #N`.

   This is the upstream answer to the bot's `<!-- pr-spec-sync:no-spec-link -->`
   reminder — answering it at PR creation keeps the bot silent.

   **Always pass `body` as a plain inline string.** Never wrap it in shell
   heredoc syntax (`$(cat <<'EOF'...EOF)`) — that is Bash substitution and
   produces a literal `$(cat <<'EOF'...` in the PR description when passed
   to an MCP tool parameter.
3. Subscribe to PR activity so CI failures and review comments are auto-fixed.

Skip this for branches that don't start with `claude/` (local/manual work).

## Per-crate context

Each subsystem has its own CLAUDE.md or `.claude/` directory with
subsystem-specific instructions:

- `crates/onsager-spine/CLAUDE.md`
- `crates/stiglab/.claude/`
- `crates/synodic/.claude/`
