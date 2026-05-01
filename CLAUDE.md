# Onsager

AI factory stack — monorepo for the Onsager event bus and its subsystems.

## Architecture

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

`portal` is the **edge** subsystem — eventually the only one that hosts
public HTTP routes (dashboard API, GitHub webhooks, OAuth, credential
CRUD). Factory subsystems (`forge`, `stiglab`, `synodic`, `ising`) live
behind the seam and coordinate exclusively via the spine. Spec #222
promotes portal to first-class peer status; while that migration is in
flight stiglab still owns the bulk of the external HTTP surface.

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
- **E** (#150, PR #227): static event manifest at
  `crates/onsager-registry/src/events.rs` (75 rows, one per
  `FactoryEventKind` variant) declares producers/consumers per
  subsystem. `xtask check-events` enforces coverage, both-ends-
  declared, emit-call-sites match producers, and listener-call-
  sites match consumers; events with no in-tree consumer are
  tagged `audit_only`. Manifest exposed at `GET /api/registry/events`.
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

The seam rule above and the "Architectural drift patterns to watch" list
below are both operational projections of this value onto the seams between
subsystems. Internal-quality work that doesn't fit those projections —
interior-to-a-subsystem hygiene — is equally in scope and should be specced
and shipped on the same footing as feature work.

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

The strategy spec #131 captures the full reasoning and the six-lever plan
that made these contracts enforced. The patterns above are now caught
mechanically by `lint-seams`, `check-api-contract`, and `check-events`;
the bullets stay as a glossary of the failure modes those checks were
designed against.

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

- Rust edition 2021, rustfmt formatting, clippy with warnings-as-errors
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
3. Subscribe to PR activity so CI failures and review comments are auto-fixed.

Skip this for branches that don't start with `claude/` (local/manual work).

## Per-crate context

Each subsystem has its own CLAUDE.md or `.claude/` directory with
subsystem-specific instructions:

- `crates/onsager-spine/CLAUDE.md`
- `crates/stiglab/.claude/`
- `crates/synodic/.claude/`
