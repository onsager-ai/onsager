# Onsager

AI factory stack — monorepo for the Onsager event bus and its subsystems.

## What makes Onsager Onsager

These commitments define Onsager-the-factory's identity — the monorepo and its subsystems. They do not, in general, prescribe the internal structure of what the factory produces; downstream artifacts and deployed systems are their own viable systems with their own identities. The exception is **how** the factory operates on its work (specs as ground truth, below) — that commitment binds both Onsager's self-construction and its production discipline.

- **Event-bus factory, not service mesh.** Coordination flows through the spine (events, pg_notify, shared tables), not synchronous calls. [ADR 0001](docs/adr/0001-event-bus-coordination-model.md) first concretized this; [ADR 0009](docs/adr/0009-three-layer-pipeline.md) / [ADR 0017](docs/adr/0017-plan-compiler-three-step-algorithm.md) (both `Identity impact: yes`) reframe it: the spine is still the runtime medium but is no longer authored directly by feature subsystems. Authorship flows Spec Plan → Workflow → Execution Plan → substrate scheduler, and the scheduler is what emits on the spine. [ADR 0027](docs/adr/0027-two-process-consolidation.md) (`Identity impact: yes`, adoption ongoing) consolidates the factory to two processes — **portal** (edge) and **engine** (compile + execute) — with the spine as the only coupling between them; the multi-subsystem citizen taxonomy (ADR 0013) retires with it. Scope: factory only.
- **Artifacts are the unit of meaning.** Every persistent, lifecycle-bearing object in the factory is an artifact — internal-authored (specs, designs) and external-referenced (PRs, Issues) alike. Workflow nodes (via executors) and portal's session runner operate on artifacts; events are state-change notifications, not first-class entities. Every artifact carries first-class **provenance** ([ADR 0010](docs/adr/0010-provenance-as-substrate-first-class.md), `Identity impact: yes`) — `Deterministic` or `Uncertain`, with a `SourceTag` — that the substrate validates and propagates. Scope: factory only — products have their own ontology.
- **Specs are ground truth, code is downstream.** When spec and code conflict, the code is wrong. Specs are amended deliberately; code is amended to match. Scope: both factory self-construction and production discipline — Onsager sessions producing artifacts inherit this.
- **Internal symmetry is load-bearing.** Equivalent concepts must have equivalent shapes (names, types, error models, write paths). Asymmetry between equivalents is a defect, not a polish concern. The seam rule and three lints (`lint-seams`, `check-events`, `check-api-contract`) are the enforcement surface. Scope: factory only; applies to session-produced code once it lands in the monorepo.

Changes to the four bullets above carry an `Identity impact: yes` flag in the modifying ADR and require explicit rationale. Changes to anything below in this file default to `Identity impact: no`. The flag tracks whether an ADR touches the four bullets above, not where the change lives.

### Honoring commitment 3: claim-honesty checks

Commitment 3 binds both directions. Code drifting from spec is a bug; shipping under a Plan item without having met it is the same bug from the other side. Before claiming a Plan item done — in a session message, checkbox tick, or PR description — pass these checks. The point isn't to inflate scope; if a check exposes that the spec was wrong, amend the spec, then ship.

- **"The main path works, so it's essentially done."** Plan items are atomic — no "essentially." Either complete, or split into `N.1` (done) / `N.2` (deferred, with reason) and amend the spec before merge. "Essentially done" is narrative, not state.
- **"Tests pass, so the change is correct."** Green CI proves the suite didn't catch a bug, not that your code is exercised. Before claiming done, mutate one line of the new path locally; the corresponding test must fail. Green CI plus uncovered logic is theater.
- **"I'll handle the edge case as a follow-up."** A follow-up with no issue doesn't exist. Open a sub-issue (`Part of #N`) and narrow the PR's stated scope, or do it now. Untracked defers become permanent corners.
- **"The workaround is fine; the proper fix is too invasive."** The judgment may be right. Honesty requires one of: (a) amend the Plan item to read "workaround, because X"; or (b) open a follow-up issue and link it. Shipping a workaround under unchanged Plan wording is silent scope reduction.
- **"`cargo check` passes, so the refactor holds."** Compilation proves types align, not that behavior is preserved. Refactors carry the same proof obligation as features: tests pass and exercise the changed paths. Without that, you're shipping a hypothesis.
- **"To be done, I should test every edge case I can think of."** No — done means the spec's Test section is satisfied. If that section is too thin or too fat, the spec is wrong; amend it. Silently exceeding or shrinking the bar is the same defect as delivering less than the Plan.

### Named failure modes

The rebuttals above point at recurring shapes; naming them lets PR review, spec comments, and session messages reference them in one token. When you spot one — yours or someone else's — name it.

- **claim ≠ reality** — the umbrella name for this annex: a session asserts done while reality hasn't met the spec.
- **silent scope reduction** — shipping a workaround, partial fix, or narrowed behavior under a Plan item whose wording still describes the full scope. The defect is shipping it without amending the Plan or opening a follow-up.
- **theater coverage** — green CI plus tests that don't exercise the new code path. Catch it by mutating a line of new code; if no test fails, the coverage is theater.
- **narrative-as-state** — prose like "essentially done", "basically works", "mostly complete" in place of an atomic Plan item state. Plan items are binary: complete, or split and amended.
- **untracked defer** — a follow-up that lives only in a session message or PR description, with no issue. It evaporates at squash-merge.

These names apply symmetrically: catching them in your own draft is the same skill as flagging them in review. See [ADR 0005](docs/adr/0005-s5-governance-scales-with-scale.md) for the meta-rule on how this S5 layer evolves with operational scale.

## Operating posture: pre-launch

Onsager has not yet been launched live to users. That removes one category of scaffolding — the kind that protects users from our half-finished state. It does NOT loosen the internal discipline that produces well-built work. The point is to ship more learning per unit time, not less rigorously, and not to defer hard problems past launch.

### What pre-launch removes

User-protection scaffolding only makes sense once humans depend on us; skip it now:

- **Feature flags that only hide work from a userbase that doesn't exist yet.** Surfaces land visibly. Flags are still legitimate for in-flight A/B work or workspace-level admin choices (e.g. an owner disabling the agent) — just not for hiding incomplete surfaces from a userbase of zero.
- **Mock implementations as bridges to absent dependencies.** If spec A needs spec B and B hasn't landed, reorder so B lands first. Throwaway mocks become "bridges that ossify" — pre-launch is when the reorder is cheap.
- **"Preview" / "beta" banners** on surfaces that are the product for our internal team. We don't need to warn ourselves.
- **Bookmark / deprecation preservation** for routes nobody has bookmarked. Delete the old route and its backend handler in the same PR as the replacement. Free redirects are fine; long deprecation windows aren't.

### What pre-launch does NOT loosen

Pre-launch is not a license to ship less rigorous work. Internal discipline matters *more* at high velocity — code we touch faster needs more guardrails, not fewer:

- **Spec discipline.** Specs remain ground truth (commitment 3); pre-launch makes amending them cheaper, not skippable. Implementation without a spec is *untracked defer*, with or without users.
- **Claim-honesty.** "Done" still means the spec's bar is met, not "essentially works." *Theater coverage* (green CI, uncovered code) is theater with or without users. Plan items stay atomic; split-and-amend, don't silently reduce scope.
- **Tests.** "Nobody will hit this path yet" is no reason to skip tests — they're how we know the code works *before* we find out it doesn't. The mutation check (mutate one line of new code; a test must fail) applies regardless.
- **Review.** Velocity comes from small PRs reviewed quickly, not large PRs reviewed never. Self-review is the floor; a second pair of eyes is the norm. Squashing onto `main` doesn't retire it.
- **Root-cause fixes over band-aids.** Pre-launch is the cheapest moment to fix properly — no migration choreography, no in-flight runs, no scar tissue. A workaround under unchanged Plan wording is *silent scope reduction*; with a follow-up issue and amended Plan it's fine; with neither it's the longest-half-life untracked defer — "I'll fix it after launch" rarely holds, since launch makes the fix more expensive.
- **Internal symmetry, seam rule, file budget, lint enforcement.** `lint-seams`, `check-events`, `check-api-contract`, and `check-file-budget` stay hard-fail. Structure rots *faster* pre-launch, not slower.
- **Identity commitments** (the four bullets atop this file). Those define the factory, not its launch status.

### What pre-launch is for

The window only opens once. Use it for moves cheap now and expensive later:

- **Reordering work** when a dependency surfaces — nobody depends on the current sequence.
- **Schema changes and migrations** without choreography — no production data, no zero-downtime constraint.
- **Discovering wrong abstractions** and rewriting them — no public contract, no client code to coordinate with.
- **Replacing whole subsystems** when the original guess was wrong — no scar tissue, no on-call to brief.

Those are *structural* moves. Quality shortcuts are not — they get more expensive at launch, not cheaper. **Pre-launch removes user protection, not work-quality protection.**

### Flipping the posture

When we launch, delete this section and replace it with the post-launch operating bars (bookmark preservation, deprecation windows, feature-flag gating for high-blast-radius surfaces, mock-implementation policy for unmerged deps, communication discipline for breaking changes). The flip is itself an ADR-worthy moment — landing live is a commitment, not a deploy event.

## Architecture

See § What makes Onsager Onsager (above) for the identity commitments these choices instantiate, and the ADRs under [`docs/adr/`](docs/adr/) for the *why* behind any rule ([`docs/adr/README.md`](docs/adr/README.md) is the current index). (`docs/architecture.md` predates the ADR 0009–0026 "0.2 refoundation" and is stale; trust this file and the ADRs.)

The shared PostgreSQL `events` / `events_ext` table + `pg_notify` channel is still the single runtime coordination medium — components stay runtime-decoupled and coordinate through stigmergy (indirect signals via the shared medium), not direct calls. The `onsager` dispatcher is a ~100-LOC CLI with zero business deps that discovers subsystem binaries on `PATH` (`scheduler`, `trigger`).

[ADR 0002](docs/adr/0002-process-product-isomorphism.md)'s two-loop framing — the **inner loop** (spec → PR → merge) and the **outer loop** (observe drift → propose rule → activate → modify the inner loop) — is superseded as a mechanism by the observer citizen (ADR 0013) but kept as a principle: every factory primitive ships with its dev-process counterpart, and every durable dev-process pattern is evidence for a future primitive.

### The 0.2 substrate: a three-layer pipeline

What changed in the 0.2 refoundation (ADRs 0009–0018) is *what authors the spine*. The old feature subsystems (forge, ising, the planned refract) no longer emit business events directly; authorship now flows through three named layers, isomorphic to a query compiler ([ADR 0009](docs/adr/0009-three-layer-pipeline.md)):

| Layer | Role | Analog |
|---|---|---|
| **Spec Plan** | The external contract — a DAG of `SpecRef` + `SpecDep`. Authored by humans (GitHub issues via webhook) or the dashboard chat (MCP). | Logical plan |
| **Workflow** | A reusable template (node graph + IO contract) for one spec `kind`; library-resident, authored once per kind. | Template |
| **Execution Plan** | An immutable, fully-resolved node graph the scheduler runs. | Physical plan |

The **Plan Compiler** ([ADR 0017](docs/adr/0017-plan-compiler-three-step-algorithm.md)) is the only thing that turns a Spec Plan + Workflow Library into an Execution Plan — three steps (lookup → instantiate → connect) plus a final validation. It is pure, deterministic, LLM-free; all non-determinism lives inside node executors. The **substrate scheduler** (`onsager-scheduler`, replacing forge's tick loop) is the only thing that executes an Execution Plan and the only emitter of business events on the spine.

Two more 0.2 pillars:

- **Executor catalog, not `NodeKind`** ([ADR 0012](docs/adr/0012-executor-catalog-replaces-nodekind.md)). Nodes carry `executor: Box<dyn Executor>`; impls live side-by-side in `onsager-nodes` (`script`, `agent`, `verify`, `subworkflow`, `human`, …) and register in an `ExecutorRegistry`. Adding one is one file, no schema migration. **Verify** is kernel-special: the *only* executor allowed to upgrade an artifact's provenance from `Uncertain` to `Deterministic` ([ADR 0010](docs/adr/0010-provenance-as-substrate-first-class.md)).
- **Observers, the second substrate citizen** ([ADR 0013](docs/adr/0013-observer-as-second-substrate-citizen.md), replacing ising). Non-blocking, cannot mutate state, read the spine, emit typed `QualitySignal` / `Insight` / `Alert` into `observer_outputs`. VSM S3* (audit), structurally separate from the workflow citizen's S1/S2/S3.

**Five kernel invariants** ([ADR 0018](docs/adr/0018-five-kernel-invariants.md)) are the substrate's constitutional check, run statically at Execution Plan compile time (`onsager-substrate/src/validate.rs`): (1) `requires_deterministic` edges reject `Uncertain` upstreams; (2) `Uncertain` is contagious except through Verify; (3) a workflow's declared `OutputSpec` provenance matches its actual exit-path provenance; (4) every `SubWorkflow` `workflow_ref` resolves (no cycles); (5) single writer per artifact. A workflow violating one doesn't load.

### Remaining subsystems and the edge

No factory subsystem remains behind the seam: `forge` is retired (→ substrate scheduler), `synodic` retired (ADR 0027 / #582 — governance returns, when real, as in-process Verify checks), `stiglab` retired (ADR 0027 / #583 — the agent control plane folded into portal's in-process session runner), `ising` deprecated (→ `onsager-observers`). The factory is two processes — **portal** (edge) and the **substrate scheduler** — over the shared spine.

**Architectural invariant**: subsystems must NOT import each other or be statically linked into the same binary. The substrate library crates (`onsager-substrate`, `onsager-nodes`, `onsager-observers`) and the shared utility crate `onsager-agent-spawn` sit below the seam and may be depended on by any side — a utility crate isn't a subsystem.

`portal` (`onsager-portal`) is the **edge** subsystem — the only one hosting public HTTP routes (dashboard API, GitHub webhooks, OAuth, credential CRUD, the MCP server). The route-level move landed via spec #222; the process-level move via [ADR 0006](docs/adr/0006-edge-dispatcher-as-the-public-boundary.md) (spec #283 — Caddy in front of portal). ADR 0008's `/agent/ws` control plane retired with stiglab (ADR 0027 / #583): portal runs agent sessions in-process (`session_runner.rs`), so there is no agent WebSocket surface at all.

In production the externally-reachable process is Caddy (the edge dispatcher), bundled in the same image (`deploy/onsager.Dockerfile`). Portal binds `127.0.0.1:3002` and owns every external route; the substrate scheduler runs alongside with no HTTP surface.

## MCP server + public skills bundle

[ADR 0007](docs/adr/0007-tools-and-skills-as-the-public-contract.md) names the **protocol shape** of clause 1 for AI runtimes: portal hosts an **MCP server** at `POST /mcp/messages` (JSON-RPC 2.0 over HTTP), and a sibling **public skills bundle** at `onsager-ai/onsager-skills` packages the operating-procedures knowledge pairing with the tools.

The MCP server is portal's clause-1 surface for AI clients (Claude Code, Cursor, Codex, custom agents, *and* the dashboard chat — one MCP client among many). The same workspace-scope auth (`AuthUser` + `require_workspace_access`) gates both REST and MCP. Tools delegate to the same DB helpers the REST handlers use — no new business logic, just typed wrappers.

Tool schemas SSOT: derived from Rust serde structs via `schemars` (`#[derive(JsonSchema)]`) — no hand-written JSON Schema, no parallel-source drift. A TS-side counterpart from the same structs is a follow-up.

The pieces (landed in stages under #288; ADR 0007 has the full history):

- **Backend** — MCP tools live in `crates/onsager-portal/src/mcp/` (action + diagnostic tools, schemas from `schemars`).
- **Skills bundle** — public user-facing skills live canonically in `onsager-ai/onsager-skills` (`npx skills add onsager-ai/onsager-skills`); cross-repo dev-process skills (`issue-spec`, `ci-triage`, …) live in `onsager-ai/dev-skills` (`npx skills add -g onsager-ai/dev-skills --skill '*' -a claude-code`). See each repo's `README.md` for the trigger-phrase matrix.
- **Dashboard client** — the dashboard chat is a same-origin MCP client; it routes every mutation call through `HitlCard.tsx` and every read-only call through a plain info block. `xtask check-hitl-coverage` hard-fails on drift between the Rust registry and the dashboard bindings.

`xtask check-tools-and-skills` enforces ADR 0007's dev-process clause (every public tool has a skill grant; every grant references a real tool). It's part of `just lint` and runs in CI via a transient `git clone` of `onsager-ai/onsager-skills` (spec #323). For local two-checkout dev, set `ONSAGER_SKILLS_DIR=../onsager-skills just lint`; otherwise `just lint` clones the sibling into `target/onsager-skills/` automatically.

**Shared skill editing.** Skills installed under `.claude/skills/` via `npx skills add onsager-ai/onsager-skills` are read-only copies — don't edit them directly (a `PreToolUse` hook, `.claude/hooks/check-skill-edit.sh`, blocks edits to any skill dir with a `.upstream-source` marker, since a future `npx skills add` would silently overwrite the change). To change a shared skill: edit it in `onsager-ai/onsager-skills`, open + merge a PR there, then re-run `npx skills add onsager-ai/onsager-skills` here.

## The seam rule (canonical)

> HTTP APIs exist only at external boundaries: **user-facing endpoints** called by the dashboard, and **webhooks** called by external services (GitHub, etc.).
>
> The external HTTP boundary is owned by `portal` (the edge subsystem). Factory processes coordinate **exclusively** via the spine: events on the bus + reads against shared spine tables. No subsystem makes HTTP calls to another subsystem. No subsystem imports another subsystem's crate. (Substrate library crates — `onsager-substrate`, `onsager-nodes`, `onsager-observers` — and the `onsager-agent-spawn` utility sit below the seam and may be shared by any side.)

This is the rule. ADR 0001 set it; [ADR 0004](docs/adr/0004-tighten-the-seams.md) made it machine-checkable via a six-lever plan (spec #131, all landed and CI-enforced via `lint-seams`, `check-api-contract`, `check-events`); the seam rule is now mechanical, not review-time discipline. [ADR 0018](docs/adr/0018-five-kernel-invariants.md) partially supersedes ADR 0004: the six code-level seam levers remain in force for cross-subsystem discipline, but the substrate's *internal* correctness contract is now the five kernel invariants (above). Note `lint-seams` currently scopes the cross-subsystem checks to `ising` (the last subsystem-shaped crate); portal is the edge and the substrate hosts (`scheduler`, `trigger`) sit below the seam.

[ADR 0006](docs/adr/0006-edge-dispatcher-as-the-public-boundary.md) closes the process-level half of clause 1 (Caddy as edge dispatcher; portal owns 100% of the external HTTP surface). ADR 0008's loopback agent control plane retired with stiglab (ADR 0027 / #583).

The two rules a session still needs day-to-day: the static event manifest at `crates/onsager-registry/src/events.rs` has one row per `FactoryEventKind` variant, each either **real** (non-empty `consumers`) or **diagnostic-only** (`diagnostic_only: true` + a `reason`) — `check-events` rejects anything else and verifies call-sites match; and `check-api-contract` requires every backend route to have a dashboard caller (or an allowlisted external-only reason) and vice versa.

## Internal aesthetic

Care about the inside the same way you'd care about the outside — the wires inside an Apple product are dressed even though no user opens the case. The seams between subsystems, the shape of internal modules, the consistency of names and errors, the absence of dead wires are first-class quality, not cleanup chores deferred until "after the feature lands."

This is a value, not a checklist. Three operating principles fall out of it:

- **Internal symmetry is a feature.** Two things that are *the same concept* should have the same shape — same name, type, error model, write path. Asymmetry between equivalents (`TriggerKind` here, `TriggerSpec` there; one creation path setting `current_version=0`, another `=1`) is a defect, even when nothing user-visible breaks.
- **No dangling wires.** `#[allow(dead_code)]` "for later," event types with no consumer, endpoints with no UI caller, compat aliases with no removal date — all the same defect: a wire connected at one end. Finish it in the same PR, or remove the loose end.
- **The inside is reviewable.** Files past ~500 LOC, modules mixing unrelated concerns, error types that change shape across a subsystem boundary — not style preferences but a tax on every future reader. Splitting and unifying them is real work, worth scheduling.

### File budget

`xtask check-file-budget` (wired into `just lint` and CI) enforces an **8000 prod-token ceiling** per `.rs` / `.ts` / `.tsx` file. "Prod" strips Rust test blocks (`#[cfg(test)]`) and skips TypeScript test files (`*.test.ts`, `__tests__/`).

Tokens are counted with `tiktoken-rs` against `o200k_base`, vendored at `xtask/assets/o200k_base.tiktoken` for offline determinism — within ~10% of Claude's tokenizer, stable across machines.

To **exempt** a file that legitimately exceeds the ceiling (out-of-scope for an active spec, binary entrypoint, generated file), place `// budget-allow: <non-empty reason>` anywhere in it. Reason text is mandatory and grep-able. Mirrors the `// seam-allow:` shape.

**Ratchet plan.** The stated value (~500 LOC ≈ ~5000 tokens) is tighter than the current 8000 ceiling. Once all exempted files are split or justified, a follow-up spec tightens the ceiling to 5000–6000. (Spec #261 set 8000 as the initial floor; ratcheting is a separate spec.)

The seam rule above and the "Architectural drift patterns to watch" list below are both operational projections of this value onto the seams between subsystems. Interior-to-a-subsystem hygiene is equally in scope and should be specced and shipped on the same footing as feature work.

### Dashboard API types: Rust is the SSOT

Portal's serde structs are the single source of truth for the wire shapes the dashboard consumes. `#[derive(ts_rs::TS)]` on a portal struct emits a committed `.ts` file under `apps/dashboard/src/lib/api/generated/` when `cargo test` runs (export dir wired in `.cargo/config.toml`); `xtask check-generated-types` regenerates + diffs against the committed tree, drift is hard-fail. Don't hand-write types the dashboard could import from `generated/`. (Companion to the `schemars` SSOT for MCP tool schemas; migration is ongoing — see `crates/onsager-portal/CLAUDE.md`.)

## User-facing vocabulary (canonical 4 nouns)

Per spec #286, the dashboard, public API field names, route segments, button copy, page titles, and user-visible docs use exactly four top-level nouns. Anything else is internal-only or surface-internal (visible only inside a workflow/run drill-down, never a top-level nav noun).

The four canonical nouns:

- **Workflow** — the automation unit (trigger + ordered stages + prompts). `/api/workflows`; spine `workflows` / `workflow_stages` (Lever D).
- **Run** — one execution of a workflow against an artifact. `/api/workflows/:id/runs`; has a status and a sequence of stage outcomes.
- **Artifact** — what a run produces (issue, PR, deployment, etc.). The core noun; `/api/spine/artifacts`; spine `artifacts`.
- **Stage** — a step within a workflow definition (gate kind + parameters). A workflow's structural unit; never a top-level nav noun.

**Sanctioned carve-out — "Plans" (spec plans).** ADR 0023 made the authored spec-plan graph a first-class user concept; ADR 0025 / spec #514 require a noun-surface launch path so chat isn't the sole way to run one. The dashboard carries a fifth nav noun, **Plans** (`/workspaces/:slug/spec-plans`), listing persisted spec plans with a HitlCard-routed "Run" button. A deliberate exception under the four-noun rule, not a violation: a spec **plan** (a substrate authoring artifact — a DAG of specs run in dependency order) is distinct from the internal-only dev-process **spec** (a GitHub issue) demoted below. Scoped to this one surface; new nav nouns still default to "no" and need their own ADR.

**Demoted to internal-only.** These stay rich in Rust / migration / spine vocabulary but never surface to users:

- **shaping** — legacy term for agent-session dispatch; retired with stiglab (ADR 0027 / #583). Survives only in ADRs and archived specs.
- **bundle / sealed / ArtifactVersionId** — internal storage terms; the user-facing concept is "artifact version".
- **spec** — dev-process term for a GitHub issue with implementation intent. Lives in CLAUDE.md and the `issue-spec` skill; never surfaces in the dashboard.

**Demoted to surface-internal.** Visible only inside a workflow / run drill-down, never a top-level nav item:

- **gate / verdict** — control points within a stage; visible in workflow detail and run history.
- **governance** — the audit/escalation surface, subsumed into run history's verdict view.
- **session** — a stage execution context; visible as the "agent-session" stage gate kind and as a drill-down from a run's stage history.
- **node** — infrastructure; visible only in settings.
- **issue** — the GitHub issue that triggered or was produced by a run; an artifact kind, not a separate concept.

**Enforcement is doc-only.** Dashboard tsx is too varied for a useful grep-based vocabulary lint, and the 2026-05-09 audit found no significant leakage. Doc commitment plus PR review is the mechanism; a lint can be added later if drift recurs. This vocabulary is for user-facing surfaces only — seam-level / internal vocabulary stays rich per "Internal aesthetic" above.

## Architectural drift patterns to watch

Loose runtime coupling is correct and stays — but the seams it creates are informal. When designing or reviewing, prefer **unification at the seam** over a bridge; collapse a bridge in the same PR that introduces it. All are caught mechanically (`lint-seams`, `check-api-contract`, `check-events`, and the `artifacts_external_ref_no_provider_fields` CHECK constraint); the list is the glossary of failure modes those checks target.

- **Parallel schemas across subsystems.** Two subsystems persisting their own version of one concept. The spine wins — collapse the private table into the spine table with a `workspace_id` discriminator. Mirror/translator is a bridge, not a destination.
- **Producer with no consumer.** New event types are a contract: producer + consumer + deploy land together, or the producer waits.
- **In-memory caches drifting from the bus.** Default to reading the spine; cache only with an explicit invalidation path tied to a spine event.
- **Half-wired API/UI contracts.** Backend and dashboard changes for one surface land in one PR (or two with a contract test that fails until both sides exist).
- **Divergent shapes from multiple write paths.** Either both paths produce the same shape, or the read side is defensive in one named place — never at every call site.
- **Compat aliases that ossify.** Land a rename and its alias removal in the same PR, no removal-date window. `lint-seams` hard-fails on new `serde(alias)` and legacy type aliases.
- **Denormalized external state.** Copying an externally-authored field (GitHub, …) into the spine creates a drift surface. Per spec #170, external-origin artifacts are **reference-only**: the spine row carries identity (`external_ref`), *our* derived lifecycle (`state`, `current_version`, `last_observed_at`), and *our* relationships — nothing the external system owns. Provider-authored fields (title, body, labels, author) are hydrated live via a portal proxy. New integrations ship a proxy, not a denormalizer; the CHECK constraint rejects a provider title in `artifacts.name` alongside a non-NULL `external_ref` at INSERT time.

### Occam's-razor checklist (spec #275)

The patterns above share one shape: a wire connected at one end. The seam levers caught seam-level instances; spec #275 adds xtask lints for the **interior** ones — speculative abstractions and untracked defers inside a single subsystem (same warn-then-ratchet rollout as `check-file-budget`). Each escapes with `// occam-allow: <reason>`.

- **`xtask check-orphan-crates`** — flags library crates (non-`[[bin]]`) with zero in-tree reverse deps. Catches dead skeleton crates on day one.
- **`xtask check-single-impl-traits`** — flags `pub trait` defs with exactly one implementor in the workspace (counting test impls so a mock doesn't false-trigger). A one-impl trait is a speculative seam; inline it or wait for the second impl.
- **`xtask check-deferred-todos`** — flags `TODO` / `FIXME` / `#[allow(dead_code)]` / `// phase N` / `// vN.M` without a same-line `#NNN` issue reference. Every "for later" must point at a real spec.
- **`xtask check-events`** also surfaces diagnostic-only event rows without a `tracking_issue` (warn-mode), so dangling skeletons in the manifest get re-examined.
- **`xtask check-adr-adoption`** (ADR 0024 / 0025, spec #506) — flags `Accepted` ADRs whose `## Adoption checklist` still has unchecked `- [ ]` items. Mark the ADR `Adoption: ongoing` while implementation is in flight, then flip to `enforced` and tick the boxes as work lands.

All five land in warn mode initially; a follow-up spec ratchets them to fail once the floor is clean.

## Workspace layout

```
crates/
  onsager-artifact/    <- domain value objects (Artifact, ArtifactId, ArtifactVersionId, Kind, Provenance, SourceTag, lineage, quality)
  onsager-spine/       <- event bus client (EventStore, Listener, Namespace, FactoryEvent) + shared spine tables
  onsager-registry/    <- type registry, seed catalog, evaluators, static event manifest, trigger registry
  onsager-github/      <- typed GitHub API + webhook verification + OAuth (single home for GitHub HTTP)
  onsager-substrate/   <- 0.2 kernel: Workflow template, Executor trait, WorkflowLibrary, Plan Compiler, five kernel invariants
  onsager-nodes/       <- executor runtime: async Executor impls (script/agent/verify/subworkflow/human), ExecutorRegistry, Scheduler
  onsager-observers/   <- Observer citizen (VSM S3* audit) — replaces ising
  onsager-agent-spawn/ <- shared agent-session spawn wiring (utility crate; portal + scheduler both depend on it, seam-safely)
  onsager/             <- dispatcher CLI (~100 LOC, no business deps)
  onsager-portal/      <- edge subsystem — public HTTP, dashboard API, GitHub webhooks, OAuth, credentials, MCP server (lib + bin)
  onsager-scheduler/   <- substrate scheduler host: trigger.fired → Plan Compiler → executor dispatch (lib + bin)
  onsager-trigger/     <- manual + replay trigger CLI — emits TriggerFired to the spine (bin)
  ising/               <- DEPRECATED — superseded by onsager-observers (ADR 0013); not in the dispatcher set
apps/
  dashboard/           <- React UI (workflows, runs, artifacts, spec plans, governance)
  marketing/           <- marketing site
```

Crate → support-crate dependencies (no crate depends on a sibling subsystem):

- `onsager-substrate` → `artifact`; `onsager-nodes` → `{artifact, substrate}`; `onsager-observers` / `onsager-registry` / `onsager-github` → `{artifact, spine}`
- `onsager-portal` (edge) → `{artifact, spine, github, registry, substrate}`; `onsager-scheduler` → `{artifact, spine, substrate, nodes, agent-spawn}`; `onsager-trigger` → `{spine, registry}`

## Getting Started

Prerequisites: Docker, Rust toolchain (via rustup), pnpm.

```bash
cp .env.example .env            # configure environment (reference for docker-compose)
just dev                        # start Postgres, run migrations, and launch services
just smoke-test                 # verify everything works (in another terminal)
```

To run agent sessions, add your `CLAUDE_CODE_OAUTH_TOKEN` via Dashboard > Settings > Credentials (encrypted at rest, passed to agents as env vars).

Services (`just dev` launches portal and the substrate scheduler):
- **Dashboard**: http://localhost:5173 (Vite dev server, HMR)
- **Portal (edge)**: http://localhost:3002 (dashboard API, GitHub webhooks, OAuth, credentials, MCP server, `/agent/ws`)
- **onsager-scheduler**: substrate scheduler (no HTTP — watches the spine for `trigger.fired`)
- **Postgres**: localhost:5432 (event spine)

Stop with `Ctrl+C` for services, `just dev-down` for Postgres.

### Parallel dev environments (per-worktree slots)

When two agents (or a human + an agent) run the stack on the same VM at once, the **slot system** (#194) gives each worktree a private, fully-containerized copy on a disjoint port block. Slot 0 is the main checkout (`just dev`, today's ports); slots 1..=99 use a 10-port stride from 9000.

```bash
just worktree-new feat-a       # branch + slot + compose project, all up
just worktree-list             # see slots, ports, container status
just slot-exec feat-a cargo test -p onsager-portal   # one-off command in the slot
just worktree-tunnel feat-a    # SSH `-L` flags for laptop access
just worktree-up feat-a        # bring an existing slot back after reboot
just worktree-rm feat-a        # tear down + remove worktree (keeps branch)
just worktree-rm feat-a --with-branch   # also delete the branch
```

The slot's edge port serves the dashboard and reverse-proxies the backend APIs same-origin (`/api/...` → portal), so the dashboard makes relative-path fetches with no per-environment URL config and no CORS surface. SSH-forward `localhost:9010` (slot 1's edge), open it, done. See [spec #194](https://github.com/onsager-ai/onsager/issues/194).

## Build & Test

```bash
just build           # Rust workspace + dashboard
just test            # All tests
just test-all        # All tests including spine integration tests
just lint            # fmt + clippy + eslint
```

(These wrap the standard `cargo build/test/clippy/fmt --workspace` and the dashboard's pnpm scripts.)

## Conventions

- Rust edition 2024, rustfmt, clippy with warnings-as-errors
- thiserror for library errors, anyhow for application errors
- Small focused commits, imperative mood, under 72 characters
- Unit tests co-located in `#[cfg(test)]` modules
- Internal deps use `path = "../..."` — no git deps, no crates.io

## Merge policy

- **PR → main: squash only.** Merge and rebase-merges are disabled at the repo level; the merge button only offers squash. One PR = one commit on `main`.
- **main → PR: rebase, not merge.** Update a PR branch with `main` via rebase (locally or the "Update branch" button) to keep history linear and avoid merge commits that get squashed away anyway. Local: `git pull --rebase origin main` (or set `git config --global pull.rebase true` once).

## Environment variables

Subsystem-specific env vars worth calling out:

- Verify fail policy — what the Verify executor does when a check fails. One of `escalate` | `deny` | `allow` (`FailPolicy` in `crates/onsager-nodes/src/verify.rs`): `escalate` parks the run non-blockingly (default); `deny` hard-fails the run; `allow` is legacy fail-open and must be opted into explicitly. Set per-node in workflow config, not via env var (the governance subsystem and its `SYNODIC_FAIL_POLICY` env var retired with ADR 0027).

## Code exploration: codegraph vs grep

A gitignored `.codegraph/` index is available (`codegraph_explore`/`callers`/`impact` MCP tools + a `codegraph` CLI) — use it to **orient and traverse**. But it's an *unsound* tree-sitter approximation (silent false negatives on method/trait dispatch, fuzzy on overloads), so **grep stays the source of truth for completeness** — every caller before a rename. See the `codegraph` skill.

## File editing (Claude Code tools)

Prefer `Edit` over `Write` for existing files; a `Write` rewrite over ~150 lines can stall on a stream idle timeout (no retry) and leave the file half-written. Split big rewrites into a small `Write` + `Edit` follow-ups.

## Session defaults (Claude Code cloud)

If the current branch name starts with `claude/` (the prefix cloud sessions create), treat PR creation and CI auto-fix as part of finishing the task — do not wait to be asked:

1. Push the branch.
2. Open a pull request. **Before calling `mcp__github__create_pull_request`, answer the spec-vs-trivial gate** (the same gate `pr-spec-sync.yml` enforces) and bake the answer into the PR at creation:
   - If a spec issue exists or you should write one, include `Closes #N` or `Part of #N` in the body.
   - If the change is genuinely `trivial` (typo, doc-only, formatting, one-line obvious fix — see `onsager-dev-process`), pass `labels: ["trivial"]`.
   - Default is spec, not trivial. When in doubt, create the spec issue first via the `issue-spec` skill, then open the PR with `Closes #N`. This answers the bot's `<!-- pr-spec-sync:no-spec-link -->` reminder up front and keeps it silent.

   **Always pass `body` as a plain inline string** — never shell heredoc (`$(cat <<'EOF'...EOF)`), which is Bash substitution and lands a literal `$(cat <<'EOF'...` in the PR when passed to an MCP tool parameter.
3. Subscribe to PR activity so CI failures and review comments are auto-fixed.

Skip this for branches that don't start with `claude/` (local/manual work).

## Per-crate context

Several crates carry their own CLAUDE.md with crate-specific instructions: `onsager-spine`, `onsager-portal`, `onsager-registry`.

## Worktree dev stack (Traefik / devproxy)

In addition to the slot system above, each git worktree can run the full
containerized stack via the root `docker-compose.yml` (direnv/.envrc set
`COMPOSE_PROJECT_NAME` + `DEV_HOST`). No host ports are published — the
caddy `edge` service joins the shared `devproxy` network and Traefik
routes by Host header. The instance is reachable at
`http://$DEV_HOST:8000` (e.g. `http://feat-x.onsager.localhost:8000`).
Agents can verify changes themselves with:

```sh
curl -s -H "Host: $DEV_HOST" localhost:8000
```

Manage worktrees with `wt new <branch>` / `wt rm <branch>` / `wt ls`
(see `~/bin/wt`).

**Host-run mode (#579, the fast loop).** `just dev` is also a devproxy
citizen: it allocates free ports per service (canonical 3000–3002/5173
first, random fallback so parallel worktree stacks never collide),
derives `DATABASE_URL` from the db's OS-assigned loopback port
(`docker compose port db 5432` — the compose publishes `127.0.0.1::5432`,
no override needed), and self-registers `Host($DEV_HOST)` with Traefik
by writing `~/dev-proxy/dynamic/$COMPOSE_PROJECT_NAME.yml` (removed on
exit). The host stack is then reachable at the same
`http://$DEV_HOST:8000`, HMR included. One-time machine setup: the
`~/dev-proxy` Traefik needs `--providers.file.directory=/dynamic
--providers.file.watch=true`, a `./dynamic:/dynamic:ro` mount, and
`extra_hosts: [host.docker.internal:host-gateway]`. Without that dir
(or outside direnv) `just dev` skips registration and behaves as before.
