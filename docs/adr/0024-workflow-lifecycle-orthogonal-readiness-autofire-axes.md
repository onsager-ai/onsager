# ADR 0024 — Workflow lifecycle: orthogonal readiness/autofire axes + trigger-neutral binding

- **Status**: Accepted
- **Date**: 2026-05-29
- **Identity impact**: no
- **Adoption**: ongoing
- **Tracking issues**: #506 (umbrella spec). Supersedes the `(active, install_id)` status-derivation introduced under ADR 0019 / spec #456.
- **Supersedes**: none at the ADR level. The `(active, install_id)`-derived `WorkflowStatus` is a code shortcut taken under ADR 0019; this ADR replaces that shortcut, not ADR 0019's IA decision.
- **Superseded by**: none

## Context

The dashboard surfaces a four-state workflow lifecycle — Drafted / Bound / Active / Paused (ADR 0019 filter chips, spec #456). The spine has **no discrete status column**. `WorkflowStatus` (`crates/onsager-portal/src/workflow_db.rs` L248-281) derives all four states from two booleans, `(active, install_id)`:

- `Drafted` = `active = false AND (install_id IS NULL OR install_id = '')`
- `Bound` = `active = false AND install_id IS NOT NULL AND install_id <> ''`
- `Active` = `active = true`
- `Paused` = the literal SQL predicate `"false"` — a dead arm that never matches a row

Spec #456's own Open Question flagged the gap: *"Does the spine already track `workflows.status ∈ {draft,bound,active,paused}` on a single column? … confirm the other three pin to the same column or come from a join."* They never did. The four chips were wired against a column that does not exist, and the derivation papered over it. Three defects follow:

1. **`Drafted` is unreachable.** Portal's create handler requires `install_id` (`crates/onsager-portal/src/handlers/workflows.rs` L213-214) and the domain type (`crates/onsager-portal/src/workflow.rs:98`) types it `i64`, non-optional. No write path produces an empty `install_id`, so the `Drafted` predicate matches zero rows through the only writer.
2. **`Bound` leaks GitHub into a generic lifecycle concept.** `install_id` is a GitHub App installation id. Defining "bound" as "has a non-empty install_id" means a Telegram / Manual / Schedule workflow can *never* be Bound — it has no install. The lifecycle vocabulary silently became GitHub-specific.
3. **`Paused` is a dangling wire.** A status the UI offers that the backend can never produce.

The binding `install_id` describes is real, but it lives at the wrong altitude. The authoritative binding is workspace/account level in `github_app_installations` (`crates/onsager-portal/migrations/007_github_app_installations.sql`: `workspace_id × install_id` UNIQUE × account), and the repo layer is already modelled by `projects` (`crates/onsager-spine/migrations/004_workflows.sql` L43-57, column `github_app_installation_id`). The same migration denormalizes `install_id` onto every workflow row (L79, `install_id TEXT`, nullable) — a copy of an account-level fact stamped onto each preset and then promoted to *define* lifecycle status. This is the "denormalized external state" and "divergent state from multiple write paths" drift the root `CLAUDE.md` enumerates, applied to the lifecycle model.

Triggers, by contrast, are already trigger-source-neutral at the substrate. `TriggerKind` (`crates/onsager-spine/src/trigger.rs:37`) is plural and GitHub-agnostic: `GithubIssueWebhook`, `GithubPullRequestClosed`, `GithubWorkflowRunCompleted`, `TelegramWebhook`, `Cron` / `Delay` / `Interval`, `SpineEvent` / `PgNotify` / `OutboxRow`, `Manual { name }` (`crates/onsager-trigger/src/main.rs`), `Replay`. Substrate, nodes, and scheduler do **not** import `onsager-github`; the GitHub coupling is edge-only. The status model is the one place GitHub leaks into generic semantics.

## Decision

Replace the derived status with two **explicit, orthogonal, trigger-source-neutral** columns. A workflow is a reusable preset; how *ready to run* it is and whether it *fires by itself* are independent facts.

### Two axes

- **`readiness`** — `drafted` → `ready`.
  - `ready` ⇔ the workflow has **≥1 trigger binding of any kind** (GitHub webhook / Telegram / Manual / Schedule / spine-event / …). Not GitHub-specific.
  - `drafted` ⇔ no trigger binding yet — authored, debugged in chat, not yet wired to fire.
- **`autofire`** — `off` / `active` / `paused`.
  - `off` — manual-only; the workflow runs only when a human presses "run a batch".
  - `active` — auto-fires on its trigger binding.
  - `paused` — was `active`, temporarily suspended; representable as a first-class state, not a dead predicate.

The axes are independent: a `ready` workflow may be `off` (manual-only), `active` (auto-firing), or `paused`. `drafted` workflows are `off` by construction (nothing to fire on).

### `install_id` is demoted to a trigger-binding attribute

`install_id` stops defining status. It becomes an attribute of a GitHub trigger binding, resolved through `workflow → trigger binding → project → installation` (the layers `projects` and `github_app_installations` already model). The `workflows.install_id` column may persist **only** as an optional mint cache for token resolution; it must not participate in `readiness` or `autofire`. Token minting and webhook-secret resolution (`crates/onsager-portal/src/handlers/workflows.rs` ~L748 / L775 / L828, `resolve_install_webhook_secret` ~L919) reroute through the binding → project → installation path rather than reading `workflows.install_id` directly.

### `ready` earns the run button; safety is run-time, not button-time

A `ready` workflow immediately earns the manual "run a batch" button. There is **no** separate button-time promotion gate. Run-time safety is the existing Verify / HITL (Synodic) gate that already guards every run — not a gate bolted onto the button. Adding a button-time gate would duplicate the run-time gate and re-introduce a second control point for one decision.

### Create no longer requires `install_id`

The hard `install_id is required` gate at workflow create is removed. A Manual-only, Telegram-only, or Schedule-only workflow must be creatable and must be able to reach `ready` via its (non-GitHub) trigger binding.

## Rejected alternatives

- **Keep `(active, install_id)`, just add the `paused` column.** Fixes the dead `Paused` arm but leaves `Bound` GitHub-specific and `Drafted` unreachable. The leak is structural, not a missing column.
- **One enum `status ∈ {drafted, bound, active, paused}` on a single column.** The shape spec #456 assumed. Rejected because `bound`/`ready` (do I have a trigger?) and `active`/`paused` (do I fire automatically?) are orthogonal — collapsing them onto one axis cannot express "ready but manual-only" or "ready and paused" without inventing combinatorial states.
- **Keep status GitHub-derived, special-case non-GitHub triggers.** Rejected — it spreads the integration-specific branch across every status read and violates "status must not depend on a single integration's fields."
- **Store the binding only in `github_app_installations` and join at read.** Correct for GitHub, but `readiness` must hold for non-GitHub triggers too; the readiness fact is "has any trigger binding", which is a workflow-level fact, not an installation join.
- **A button-time promotion gate (`ready` → `armed` → runnable).** Rejected per Decision 1b — run-time Verify/HITL already owns run safety; a button-time gate is a redundant second control point.

## Consequences

- `workflows` gains `readiness` and `autofire` columns; the `(active, install_id)` derivation and the dead `Paused => "false"` predicate are deleted. `active` is backfilled into `autofire`; `readiness` is backfilled from trigger-binding presence.
- The lifecycle vocabulary becomes trigger-source-neutral: a Telegram-only or Manual-only workflow reaches `ready` exactly like a GitHub one.
- `install_id` is no longer load-bearing for status. Any remaining read is a token-mint cache, resolved through the binding → project → installation path; status reads never touch it.
- The dashboard `WorkflowStatusChips` remap from the single derived axis to the two explicit axes. The FTUE funnel events (`ftue.inspected/drafted/bound/activated`, spec #404) are analytics and are **not** touched by this ADR.
- Substrate stays GitHub-agnostic — no `onsager-github` dependency is added to substrate / nodes / scheduler.

## Dev-process counterpart

Per ADR 0002, the dev-process analog of the two axes is the split between a spec issue's *readiness* (does it have an actionable Plan + a linked trigger — a label, a milestone, an assignee) and its *autofire* (is it picked up automatically by the queue, held manually, or paused). A spec with no entry condition is `drafted`; one with a trigger is `ready`; whether the queue auto-pulls it is the `autofire` axis. The orthogonality is the same: a ready spec can sit manual-only or be auto-scheduled.

## Adoption checklist

- [x] Decision recorded (this ADR), `Identity impact: no`.
- [x] Phase 1 — add `readiness` + `autofire` columns to `workflows`; backfill `autofire` from `active`, `readiness` from trigger-binding presence (transitional fallback: any trigger / install_id present → `ready`). (spine migration 007; #506)
- [x] Phase 1 — delete the `(active, install_id)` derivation and the dead `Paused` predicate from `workflow_db.rs`; `readiness` is driven by trigger-binding existence for any `TriggerKind`. `WorkflowStatus` now derives from the two axes; the dead `Paused => "false"` arm is gone. (#506)
- [x] Phase 1 — reroute token minting / webhook-secret resolution through `workflow → trigger binding → project → installation`; keep `workflows.install_id` only as an optional mint cache, out of the status path. `Workflow.install_id` is now `Option<i64>` (nullable column, NULL-tolerant read), and `activate_workflow` / `deactivate_workflow` resolve the install via `pick_install_id` — `(workspace_id, repo) → projects.github_app_installation_id`, with the cached column as fallback. (#506)
- [ ] Phase 1 — remove the `install_id is required` create gate; a Manual/Telegram/Schedule workflow is creatable and can reach `ready`. (the type + write path now accept a missing install — `install_id <= 0` maps to `None`; the remaining work is dropping the `github_issue_webhook`-only `validate_create_body` gate and accepting a structured non-GitHub trigger on the REST create body, paired with the chip UI below — tracked remainder, #506)
- [ ] Phase 1 — remap `apps/dashboard/src/components/workflows/WorkflowStatusChips.tsx` to the two axes; leave the spec #404 FTUE funnel events untouched. (tracked remainder, #506)
- [ ] Phase 3 — `ready` workflows earn the manual "run a batch" button (reuse `run_spec_plan` / `run_workflow`); no button-time gate.
- [ ] Flip `Adoption` to `enforced` and tick the boxes once Phases 1 + 3 land.

## Out of scope

- **Multiple trigger bindings per workflow.** The current schema persists one trigger per workflow; `readiness` is defined over "≥1 binding" so it already accommodates a future multi-trigger schema, but adding that schema is a separate spec.
- **Chat-surface relocation and the run-button UI placement.** Governed by ADR 0025; this ADR fixes the lifecycle model and that `ready` earns the button, not where the button renders.
- **Run-path recovery determinism.** The `register_plan` library-version pinning fix is independent (#506 Phase 2); not part of the lifecycle model.
- **Dropping the `workflows.install_id` column entirely.** Deferred until the token-mint path is confirmed to no longer need the cache; this ADR removes it from the *status* path only.
