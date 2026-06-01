# ADR 0025 — Chat as design + maintenance surface

- **Status**: Accepted
- **Date**: 2026-05-29
- **Identity impact**: no
- **Adoption**: enforced
- **Tracking issues**: #506 (umbrella spec). Amends ADR 0020 (chat as portable global component); reconciles ADR 0023 (spec-plan run orchestration).
- **Supersedes**: none. Amends ADR 0020 — see the note below; the responsive-mount + auto-scope decisions of 0020 stand.
- **Superseded by**: none
- **Amended by**: #542 — the noun surface (Spec Plans) gains a guided *authoring* affordance (a "Create Plan" form), so chat is no longer the sole spec-plan **authoring** path, just as it is no longer the sole *launch* path. This is a complement, not a reversal: chat stays the conversational/design authoring surface; the form is the deterministic path for a known plan. See § "Routine actions move to the noun surfaces".

## Context

ADR 0020 framed dashboard chat as "one MCP client among many" (ADR 0007) — a portable global component with no privileged top-level position. That framing is right at the **protocol** layer: the MCP control plane exposes intent / spec_plan / compile / run / status / artifacts / verdicts, and Claude, Cursor, the dashboard chat, and any custom agent are interchangeable consumers of it.

But the product surface drifted away from the framing in two opposite directions, and they collide:

- **Chat became undiscoverable on desktop.** ADR 0020's invocation table makes the FAB mobile-only (`apps/dashboard/src/components/chat/ChatFab.tsx` is `md:hidden`). On desktop the only entries are the top-chrome bell (`ChatBell`, `aria-label="Open Inbox"` — it opens the *Queue*, not a conversation) and the keyboard toggle `⌘J` (`apps/dashboard/src/lib/chat/use-chat-ui.tsx`). A first-time desktop user has no labeled way to *start a conversation*; the only visible affordance opens an inbox. This is the predicted output of ADR 0020's "more prominent entry → more specific scope" table, which left desktop with no general-purpose, labeled entry.
- **Chat became the *sole* launch+monitor surface for spec-plan runs.** ADR 0023 Decision 2 placed both the authored spec-plan graph (F1) and live run progress (F2) exclusively in chat, with no noun-surface alternative. So the one surface that is hard to find on desktop is also the only place a routine run can be started or watched. A user who cannot find chat cannot run a workflow.

The "N interchangeable frontends" principle (ADR 0007) is about *intent formation* through the MCP control plane. It does not require that every routine action be reachable *only* through a conversational frontend. Conflating the two produced a surface that is simultaneously privileged (sole launch path) and hidden (no desktop entry).

## Decision

Chat is the **design + maintenance** surface — not a pure interchangeable frontend, and not the routine-action surface. The boundary:

> **Actions default to the noun surfaces. Chat carries only design (drafted) + maintenance (scope=run incident handling).**

This amends ADR 0020's "chat is one interchangeable frontend, not privileged" to: chat has a *specific role* (design + maintenance) on the dashboard, while the MCP-intent-layer "N interchangeable frontends" of ADR 0007 still holds at the protocol layer. The two are not in tension once the layers are named — ADR 0007 governs the protocol; this ADR governs the dashboard's chat *role*.

### Chat's scope narrows to two jobs

- **Design (drafted).** Authoring and debugging a workflow preset — the CAD bench. This is where a `drafted` workflow is shaped before it earns a trigger binding (ADR 0024). The spec-plan authoring loop (`submit_spec_plan` / `get_spec_plan`) and its in-chat `SpecPlanDAG` (ADR 0023 F1) live here.
- **Maintenance (scope=run).** Incident handling on a specific run — the maintenance port. When something goes wrong with a run, the run-scoped chat thread is where it is diagnosed. ADR 0023's in-chat run-progress overlay (F2) stays available here as the maintenance view of a run already launched.

### Routine actions move to the noun surfaces

- A **"run a batch" button** lands on `ready` workflows (ADR 0024) and on persisted spec plans, reusing `run_spec_plan` / `run_workflow`. Starting a routine run no longer requires opening chat. Chat is no longer the *sole* launch path for spec-plan runs — this is the reconciliation with ADR 0023 Decision 2.
- A guided **"Create Plan" form** lands on the Spec Plans surface (#542), reusing `compile_dry_run` (live lint + DAG preview) and `submit_spec_plan` (routed through a HitlCard). Authoring a known plan no longer requires describing it in chat. This extends the same reasoning from *launch* to *authoring*: a deterministic, structured action belongs on the noun surface; chat remains the conversational/design path for plans that are still being shaped.

### Desktop gains a labeled chat entry

A labeled desktop chat entry (e.g. `MessageSquare` icon + "Chat" + the `⌘J` hint) opens the conversation surface at `tab=thread` — the *design/maintenance* surface, not the Inbox/Queue. The existing `ChatBell` stays as the Inbox / HITL counter (`tab=queue`); `ChatFab` stays mobile-only. The two entries are distinct: the bell is "what needs my attention" (Queue), the new entry is "let me design or diagnose" (Thread).

ADR 0020's responsive mount, contextual auto-scope, pin affordance, and storage schema are unchanged. This ADR changes chat's *role and discoverability*, not its mount mechanics.

## Rejected alternatives

- **Keep ADR 0020 as-is (chat as pure interchangeable frontend).** Rejected — it leaves chat undiscoverable on desktop and, combined with ADR 0023, makes a hidden surface the sole launch path. The framing needs the design/maintenance role named explicitly.
- **Make chat a top-level tab again.** Rejected — re-creates the mode-split ADR 0019 retired and re-privileges chat as a navigation noun. The fix is a labeled *entry*, not a tab.
- **Put the run button only in chat, but make chat discoverable.** Rejected — discoverability alone does not fix the layering error. A routine run is a noun-surface action; routing it through a conversational frontend is the wrong altitude even when the frontend is easy to find.
- **Drop the in-chat run-progress overlay (ADR 0023 F2) entirely in favour of a noun surface.** Rejected for this ADR — F2 is the *maintenance* view of a run and belongs in run-scoped chat. Folding plan runs into the Runs surface remains the deliberate follow-up ADR 0023 named; this ADR does not pull it forward.

## Consequences

- Desktop users get a discoverable, labeled way to start a conversation; the bell stops being the only (mislabeled) entry.
- Routine runs are startable from the noun surfaces without opening chat. Chat's job is legible: design and maintenance, not routine operation.
- ADR 0023 Decision 2 is amended in one respect — chat is no longer the *sole* launch surface for spec-plan runs — without disturbing its in-chat authored-graph (F1) and run-progress (F2) views, which become the design and maintenance views respectively.
- ADR 0007's protocol-layer "N interchangeable frontends" is untouched and explicitly reaffirmed.

## Dev-process counterpart

Per ADR 0002, the dev-process analog: the issue/PR thread is the *design + maintenance* surface (where a change is shaped and where incidents on it are discussed), while the merge button, the CI re-run button, and the label controls are the *noun-surface actions* — you do not type a sentence to merge a PR. This ADR brings the dashboard into line with that split: conversation for design and diagnosis, buttons for routine operation.

## Adoption checklist

- [x] Decision recorded (this ADR), `Identity impact: no`.
- [x] Phase 3 — add a labeled desktop chat entry opening `tab=thread`; keep `ChatBell` as the Inbox/Queue counter; `ChatFab` stays mobile-only. `DesktopChatEntry` (MessageSquare + "Chat" + `⌘J` hint) mounts in the desktop top chrome and opens `{ scope: workspace, tab: thread }`. (spec #514)
- [x] Phase 3 — add the "run a batch" button to `ready` workflows and persisted spec plans (reuse `run_spec_plan` / `run_workflow`); a routine run is startable without opening chat. `HitlActionButton` is the standalone HitlCard host (page button → review dialog → `mcpCallTool`); it backs the workflow "Run a batch" and the Spec Plans surface "Run". (spec #514)
- [x] Phase 3 — narrow chat default scope to design (drafted) + maintenance (scope=run); remove chat as the only launch path for spec-plan runs. `deriveAutoScope` now resolves bare routes to workspace (design) or run/verdict (maintenance) only — workflow detail folds into workspace; the Spec Plans surface is the noun-surface launch path. (spec #514)
- [x] Add an "amended by ADR 0025" note to ADR 0020's metadata block. The note names the design/maintenance role, the desktop entry, and the narrowed route → default-scope table.
- [x] Flip `Adoption` to `enforced` and tick the boxes once Phase 3 lands. (spec #514)

## Out of scope

- **Folding spec-plan runs into the Runs surface.** Remains the deliberate follow-up ADR 0023 named; not pulled forward here.
- **Chat mount mechanics.** Responsive mount, auto-scope, pin, storage — governed by ADR 0020, unchanged.
- **Notification action button → MCP elicitation binding.** Still the follow-up ADR 0020 deferred.
