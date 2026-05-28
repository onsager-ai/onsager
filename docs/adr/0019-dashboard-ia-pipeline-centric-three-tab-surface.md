# ADR 0019 — Dashboard IA: pipeline-centric three-tab surface

- **Status**: Accepted
- **Date**: 2026-05-22
- **Identity impact**: no
- **Tracking issues**: #450 (implementation spec). Refines the IA frame set by spec #289 (closed 2026-05-13 as completed, sub-PRs 2a–6 unshipped) and the FTUE umbrella spec #397 (closed 2026-05-19). Retires spec #398 (universal-chat-entry, closed 2026-05-18) as a side effect. The chat surface restructuring it implies is named in the sibling ADR 0020.
- **Supersedes**: none. The predecessor specs (#289, #397, #398) are the prior dashboard-IA frame in prose; the repo convention reserves the `Supersedes` field for ADR-to-ADR replacement, so no entry here.
- **Superseded by**: none

## Context

The dashboard IA has accreted in layers. Spec #289 set a two-surface frame (sidebar = Chat + Workflows; everything else under ⌘K). Spec #398 routed `/` to `/chat` so first-time users would see chat on landing. Spec #306 added redirect-preserving fallbacks for older top-level routes (`/sessions`, `/spine`, `/governance`, `/issues`, `/nodes`). The FTUE umbrella spec #397 (and its children #404, #406, #408) layered on activation instrumentation, template gallery, and a factory metaphor for user-facing copy.

In code today this produces:

- Two route entries pointing at the same `ChatPage` component (`/chat` unscoped, `/workspaces/:slug/chat` scoped) with subtly different storage-key behaviour (`apps/dashboard/src/pages/ChatPage.tsx:293-297`)
- A two-item sidebar carrying a workspace switcher + user menu around two nav items, occupying ~260px of chrome to serve two destinations
- A 4-tab `WorkflowDetailPage` where every tab is a stack of cards (`apps/dashboard/src/pages/WorkflowDetailPage.tsx:218-256`)
- Six redirect entries for previously top-level surfaces; the redirect graveyard signals the IA has been shuffled at least once
- Multiple banners (OSS notice, draft chip strip, MetaphorBanner, webhook warnings) stacking at the top of the chat surface

First-paint screen density does not differ visibly from a generic SaaS dashboard with two nav items. The sidebar is mostly chrome around empty space, and `ChatPage`’s default 2fr/3fr split renders an empty DAG preview as 60% of the viewport before the user has typed anything. Complexity hides in the second layer rather than the first: top-level looks light, drill-in looks dense.

The deeper mismatch is between the domain hierarchy and the navigation hierarchy:

|Object  |Domain position                     |URL position                               |
|--------|------------------------------------|-------------------------------------------|
|Run     |Child of Workflow                   |Flat `/runs/:id`                           |
|Artifact|Child of Run (or Stage)             |Flat `/artifacts/:id`                      |
|Verdict |Child of Run                        |No route; a tab inside Workflow detail     |
|Draft   |Pre-bound Workflow                  |No route; a chip strip inside ChatPage     |
|Issue   |Trigger source, not a product object|Top-level detail route `/issues/:proj/:num`|
|Project |Implicit (repo+install binding)     |No UI at all                               |

Spec #289’s two-surface framing also splits a single linear pipeline (Chat → Draft → Bind → Workflow → Run → Artifact + Verdict) into two parallel modes (R&D vs production). The end-to-end view is hidden by the navigation shape.

## Decision

Adopt a pipeline-centric three-tab top-level IA. The sidebar is removed; workspace switcher and user menu move into the top chrome.

|Tab      |Surface role                                                                               |URL                          |
|---------|-------------------------------------------------------------------------------------------|-----------------------------|
|Workflows|All workflows in current workspace, filtered by state. Default landing for signed-in users.|`/workspaces/:slug/workflows`|
|Runs     |Cross-workflow execution history, filterable by workflow / status / time.                  |`/workspaces/:slug/runs`     |
|Activity |Spine event stream at operator grain.                                                      |`/workspaces/:slug/activity` |

The Workflows tab carries filter chips for the four workflow lifecycle states — Drafted, Bound, Active, Paused. These chips expose the activation ladder (per spec #404 instrumentation, with `Drafted` as the new key metric per KB position `364d79199ca681b4953be66481634f97`) as filter dimensions, not as separate navigational surfaces. A workflow with `status='draft'` is the same object as one with `status='bound'`; only its state pin differs.

Routing changes:

- `/` redirects to `/workspaces/:slug/workflows` for signed-in users with at least one workspace, otherwise to `/workspaces` (picker)
- `/chat` and `/workspaces/:slug/chat` are retired; existing bookmarks 301 to `/workspaces/:slug/workflows`. The chat surface itself remains reachable as a global component per ADR 0020
- `WorkflowDetailPage`’s four tabs (Definition / Runs / Artifacts / Verdicts) collapse into one timeline page with anchored sections. Existing hash anchors (`#definition`, `#runs`, `#artifacts`, `#verdicts`) are preserved as scroll targets for bookmark compatibility
- Issues are not a top-level surface. The existing `/issues/:projectId/:number` route remains as a deep-link target for webhook notifications, with no in-dashboard entry point. Issue records render as Activity source-type rows and as embedded trigger-instance rows inside Workflow detail
- The redirect map from spec #306 is extended with the chat retirements but otherwise unchanged

The AI factory metaphor is removed from the dashboard’s substrate-facing surfaces. A grep of `apps/dashboard/src/` finds the metaphor at six distinct locations, not the five originally tallied under spec #408 (only one of which carries an inline comment marker):

1. `pages/ChatPage.tsx:838-843` — chat empty-state “inspection reports / QC checkpoint” copy (spec #408 location 1, explicitly commented in code)
1. `pages/WorkflowsPage.tsx:69` — empty-state “factory starts responding to GitHub events” copy
1. `pages/WorkflowStartPage.tsx:23,99,255` — page title “Start the factory” and “Run factory” button label
1. `components/factory/workflows/MetaphorBanner.tsx` — workflow detail banner; the file is deleted, not just its copy
1. `components/chat/BindDraftDialog.tsx:186` — bind-draft dialog prompt “First, name your factory floor.”
1. `lib/chat/llm-client.ts:48,67` — chat assistant system prompt persona (“AI factory operator”, “QC checkpoints” vocabulary). This surface was missed by the original spec #408 enumeration. It matters because it implicitly propagates the metaphor through every assistant response.

The following metaphor surfaces are explicitly **kept**, per KB position (Notion `364d79199ca681b4953be66481634f97`: metaphor is permitted on marketing and template-content surfaces):

- `pages/LandingPage.tsx` (marketing landing)
- `pages/ShowcaseDogfoodPage.tsx` (public showcase)
- `apps/marketing/*` (marketing app)
- `apps/dashboard/src/lib/templates/v0.json` — the `factory_framing` string on each template, locked as user-facing template copy by the FTUE umbrella spec #406. These describe each template to the user before they pick one and are content, not chrome.
- `components/layout/CommandPalette.tsx:136` — `factory` retained as a keyword alias on the “New workflow” command, so users who knew the old vocabulary can still find the action

The directory name `components/factory/` is an internal identifier that KB explicitly excludes from the metaphor-permitted list. Renaming it is a mechanical refactor (50+ import sites) and lands as a follow-up issue, not under this ADR.

The metaphor tightening therefore goes by *location*, not just by *surface type*. KB’s existing rule (no IA / routes / identifiers / DB schema) is preserved; this ADR enumerates the six dashboard locations that fall under the rule but had been missed in practice.

## Rejected alternatives

- **Keep spec #289’s two-surface IA, only fix the UI density.** Considered as the lowest-risk option. Rejected because the underlying problem is structural — chat and workflows are adjacent stages of one pipeline, not parallel surfaces — and UI-only fixes leave the mental-model mismatch in place.
- **Object-centric three-tab IA (Workflows / Issues / Activity).** Considered. Rejected because Issues are a passthrough view of GitHub’s data model, not a first-class Onsager object. Promoting them dilutes the workflow-first framing. The `/issues/:proj/:num` route stays as a deep-link target only.
- **Single-object IA (workflow-as-only-object), landing on the last workflow visited.** Considered. Rejected because OSS users with zero workflows have no surface to enter; the “where do I start” question has no answer in this shape.
- **Workspace-as-canvas single-page dashboard.** Considered. Rejected because the canvas grid does not scale past ~10 workflows, and cross-workflow runs and activity have no obvious place to land.
- **Promote Drafted / Bound / Active / Paused to separate top-level tabs.** Considered briefly. Rejected because these are workflow lifecycle *states*, not separate surfaces. A workflow does not change identity when its state pin moves from Drafted to Bound; filter chips on the Workflows tab express the same axis without fragmenting the object model. This also keeps the activation ladder (per ADR 0009-adjacent instrumentation spec #404) on its metric axis rather than promoting it to IA, in line with the KB framing.
- **Retain the factory metaphor inside the dashboard as teaching copy.** KB’s current rule would technically permit it (the rule excludes IA / routes / identifiers / schema but not in-app copy). Rejected because (a) the metaphor has spread to six unrelated surfaces with no coordinated voice, (b) the product positioning has shifted from “AI factory” to “AI-augmented work executor” per the 2026-05-14 KB amendment, and (c) once a user produces their first draft, substrate-native vocabulary should take over — also a KB-explicit transition.

## Consequences

### Positive

- First-paint screen density drops sharply. The chrome is one row (logo + workspace switcher + three tabs + ⌘K + avatar); the main area is full-width.
- Cross-workflow visibility becomes a first-class affordance. Runs and Activity answer “what happened in this workspace lately” without drilling into a specific workflow.
- Workflow lifecycle becomes legible via state filter chips. The activation ladder surfaces as filter counts (`Drafted · 2`, `Active · 3`), keeping it visible as a metric without making it a navigational axis.
- Chat has a single, consistent role across the dashboard as a global component (per ADR 0020), not a separate mode.
- In-app vocabulary becomes consistent: factory metaphor confined to marketing and template content; dashboard chrome and copy speak substrate-native terms.

### Negative trade-offs

- Existing OSS users will find their muscle memory broken on first deploy. Mitigation: changelog entry plus a one-release “what’s new” banner pointing at the new chrome.
- The Workflow detail page becomes long when a workflow has many runs, artifacts, and verdicts. Mitigation: each anchored section truncates to “last N” with `[show all]` controls and lazy-loads on scroll.
- The `factory_framing` strings inside `templates/v0.json` now exist under a different governance regime than the surrounding UI (kept, not removed). Mitigation: a comment block at the top of the JSON file cites this ADR and the FTUE umbrella spec #406.
- The Issue detail route survives as an entry-point-less deep link. Mitigation: webhook notification links still resolve; the route is documented as “deep-link target only” in the route table.
- The `components/factory/` directory name is technically a KB-violating identifier that this ADR does not resolve. Mitigation: a follow-up refactor issue is filed; until renamed, the directory remains internal-only and never surfaces to users.

### Neutral

- The redirect map from spec #306 grows by two entries (`/chat`, `/workspaces/:slug/chat`). The existing redirect plumbing handles it.
- Settings remains accessible under the user menu and ⌘K, with no top-level tab. Unchanged from prior IA.
- Hash anchors inside the collapsed `WorkflowDetailPage` stay bookmark-stable.

## Dev-process counterpart

Per ADR 0002, the dev-process analog of a top-level dashboard tab is the issue label that gates work entering each lane. Workflows tab maps to issues labelled `area:workflow`; Runs tab maps to runs surfaced through spec issues with run logs; Activity tab maps to the spine event log used by maintainers to debug cross-spec behaviour.

The lifecycle state filter chips parallel issue states: Drafted ↔ issue `draft`, Bound ↔ `in-progress`, Active ↔ `in-flight`, Paused ↔ `on-hold`.

The “metaphor only in marketing and template content” rule maps to the spec authoring convention: an issue’s title and body use substrate terms; only fields whose contents are surfaced to users (such as a template’s `factory_framing`) may carry metaphor copy.

## Adoption checklist

- [ ] Land the new top chrome (workspace switcher + three tabs + tools cluster); delete `AppSidebar.tsx` and remove its imports
- [ ] Add list routes for `/workspaces/:slug/runs` and `/workspaces/:slug/activity`; the existing flat `/runs/:id` and `/artifacts/:id` detail routes are preserved
- [ ] Collapse `WorkflowDetailPage.tsx` from four tabs to one timeline page; preserve `#definition`, `#runs`, `#artifacts`, `#verdicts` hash anchors as scroll targets
- [ ] Add state filter chips (`All`, `Drafted · N`, `Bound · N`, `Active · N`, `Paused · N`) to the Workflows list
- [ ] Add 301 redirects for `/chat` and `/workspaces/:slug/chat` in the spec #306 redirect map; extend the smoke-test for redirect coverage
- [ ] Remove the metaphor from the six locations enumerated in the Decision section; delete `MetaphorBanner.tsx`; update the `llm-client.ts` system prompt to substrate-native vocabulary
- [ ] Add a comment block to `apps/dashboard/src/lib/templates/v0.json` citing ADR 0019 and spec #406 (templates’ `factory_framing` stays as user-facing copy)
- [ ] File a follow-up issue to rename `components/factory/` to a substrate-native name; mark as blocked until ADR 0019 ships
- [ ] Add `closed by ADR 0019` follow-up comments to specs #289, #397, #398
- [ ] Update KB page `364d79199ca681b4953be66481634f97` (“Onsager 产 品定位 v0.1”) to point at ADR 0019 as the current IA reference
- [ ] Flip Status to `Accepted` once the above land

## Out of scope

- **Chat surface mount form.** Where chat renders (FAB / Sidebar / Drawer / Palette) is governed by ADR 0020, not this ADR. This ADR fixes the tab structure only.
- **Workflow detail content model.** Which artifacts and verdicts appear under each anchored section is governed by ADR 0009 + ADR 0016. This ADR changes the page’s chrome from tabs to anchored sections, not the content shape.
- **Cross-workspace surfaces.** All three tabs are workspace-scoped. A global “all workspaces” overview is not part of this ADR.
- **Mobile-specific IA.** The three-tab pattern fits desktop and tablet. On phone-narrow viewports the chrome compresses (workspace switcher + logo on row 1, tabs on row 2), and the long workflow detail page benefits from anchored sections, but no phone-specific IA changes are committed here.
- **The `components/factory/` directory rename.** Filed as a follow-up. This ADR removes the metaphor from user-visible copy and from the assistant persona, but does not refactor the internal module name.
