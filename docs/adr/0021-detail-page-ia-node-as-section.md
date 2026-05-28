# ADR 0021 — Detail page IA: node-as-section, attribute-as-expanded-detail

- **Status**: Proposed
- **Date**: 2026-05-27
- **Identity impact**: no
- **Tracking issues**: pending — implementation spec to be opened on Accept.
  Builds on ADR 0019 (which collapsed `WorkflowDetailPage`'s four tabs to
  anchored sections) by extending the same logic to the remaining three
  detail pages and by reframing the section split itself around substrate
  primitives rather than v0.1 subsystem boundaries.
- **Supersedes**: none. Refines ADR 0019 without replacing it.
- **Superseded by**: none

## Context

ADR 0019 collapsed `WorkflowDetailPage`'s four tabs (Definition / Runs /
Artifacts / Verdicts) into one anchored-section timeline. The same v0.1
framing — node attributes promoted to top-level surfaces — survives in
three sibling detail pages, and partially in `WorkflowDetailPage` itself.

The collapse was a chrome refactor; the underlying mental model
("Artifacts and Verdicts are surfaces") was not revisited.

In code today this produces:

- `pages/WorkflowDetailPage.tsx:219-256` — four anchored sections.
  `Definition` double-renders (the linear `ArtifactFlowOverview` pill
  strip immediately above a per-stage `<Card>` list of the same
  stages with `gate_kind` + in/out `artifact_kind` badges). `Runs`
  contains four sibling `<Card>`s: `ActiveRunsBanner`, `RunsList`,
  `WorkflowEventsCard`, `WorkflowSessionsCard` — but `events` and
  `sessions` are run-derived observers (ADR 0013 Observer surface),
  not workflow-scoped concepts. `Verdicts` and `Artifacts` are per-run
  attributes promoted to workflow scope as "last 5 / last 20" lists.
- `pages/RunDetailPage.tsx:142-258` — five `<Tabs>` (Overview / Stages
  / Artifacts / Verdicts / Events). The `Overview` tab duplicates the
  metadata that already lives in the page header plus the stage
  progress strip already shown in detail in the `Stages` tab. The
  `Artifacts` tab contains a single artifact link with a hardcoded
  `"Issue"` fallback for `artifact_kind` (a v0.1 GitHub-bound default,
  inconsistent with the typed artifact registry per ADR 0003).
  `Verdicts` and `Events` are stage attributes, not run-level scopes;
  treating them as top-level tabs requires the page to invert the
  natural per-stage grouping into per-attribute grouping and lose the
  binding.
- `pages/ArtifactDetailPage.tsx:329-432` — four `<Card>`s:
  `Run Lineage` (the producing run's internal DAG), `Version History`
  (versions table with session origin), `Vertical Lineage` (same
  artifact across versions — overlaps with `Version History`),
  `Horizontal Lineage` (related artifacts in the run). The three
  lineage cards are three projections of one provenance graph
  (ADR 0010); rendering them as parallel surfaces hides their shared
  origin. The page renders no content — `artifact_kind` is never
  dispatched into a body view, so an `Issue`, a `markdown`, a
  `pull_request` and a `verdict` all look identical from the dashboard.
- `pages/IssueDetailPage.tsx:1-376` — four `<Card>`s (Labels,
  Assignees, Description, Onsager metadata) plus a 4-up MetaCard grid
  (Author / Comments / Updated / Lifecycle) that mixes GitHub-fed and
  substrate-fed fields. The page is hardcoded to
  `artifact_kind=github_issue` with `projectId` and `number` URL
  parameters (lines 22-25, 31-32); the "Onsager metadata" Card name is
  the page admitting it cannot reconcile the two ontologies.

The deeper pattern: each page promotes a *node attribute* (verdict,
events, lineage axis, kind-specific fields) to *page-level section*.
This is a v0.1 "subsystem-per-tab" mental model. ADR 0019's tab → anchored-section
transformation changed the chrome but kept the framing.

A constraint imposed by ADR 0019 narrows the design space: artifact
and verdict have no top-level list view in main IA (only Workflows /
Runs / Activity tabs exist). Detail pages cannot delegate "show me
artifacts/verdicts" to a navigation surface that does not exist;
related lists must surface inline or via filtered Activity URLs.

## Decision

Adopt a uniform detail-page IA shape across Workflow / Run / Artifact
/ Issue, organized around two substrate primitives:

- A *node* is the unit a detail page is organized around: a workflow
  stage (in Definition), a run stage (in RunDetail), an artifact
  version (in ArtifactDetail Provenance).
- An *attribute* is anything that hangs off a node: output artifact,
  sessions, verdict, events, source tag, kind, gate, executor.

A *section* corresponds to a coarse substrate object class, not to a
node attribute.

Per-page shape:

|Page           |Sections                            |Node unit                                    |Notes                                                                                                                       |
|---------------|------------------------------------|---------------------------------------------|----------------------------------------------------------------------------------------------------------------------------|
|WorkflowDetail |Definition, Activity                |stage (in Definition) / run (in Activity)    |`Verdicts` and `Artifacts` sections removed. Reachable via Activity-tab filters and via run drill-down.                     |
|RunDetail      |Stages (one section)                |stage                                        |Five tabs collapsed to zero. Header strip absorbs Overview. Artifact / sessions / verdict / events hang on each stage row.   |
|ArtifactDetail |Content, Provenance, Consumers      |version (in Provenance Time axis)            |`Content` dispatched by `artifact_kind`. Provenance = single graph, axis switch (Time / Origin / Relations). Version History merges into Time axis. |
|IssueDetail    |— (page retires)                    |—                                            |All renders move into `ArtifactDetail`'s Content section under `artifact_kind=github_issue` dispatch.                       |

Five layout invariants:

1. **One section per substrate object class.** A workflow has Definition
   (its spec) + Activity (its history). A run has Stages (its only
   first-class child). An artifact has Content + Provenance + Consumers.
1. **Node attributes hang off the node, not the page.** Verdict, events,
   sessions, output artifact all render inside the expanded detail of
   their owning stage row, not as sibling sections.
1. **`<Tabs>` is not primary IA.** Tabs are reserved for orthogonal
   *views of the same data* (e.g., Provenance axis: Time / Origin /
   Relations). Tabs are forbidden for switching between substrate object
   classes. An xtask lint enforces this on `pages/*DetailPage.tsx`.
1. **Header strip absorbs single-value metadata.** Status, owning
   workflow/run link, started-at, trigger source, action buttons. No
   "Overview" section.
1. **Page = one document, not a stack of `<Card>`s.** Sections lead with
   a label + horizontal rule; no `<Card>` wrapper around section
   content. `<Card>` survives only as a contained unit for genuinely
   self-bounded sub-content (e.g., a rendered markdown artifact body).

Routing changes:

- `/issues/:projectId/:number` 301-redirects to `/artifacts/:id` via
  server-side `github_issue.external_ref` lookup (the same join
  `IssueDetailPage` already performs at lines 49-53).
- `WorkflowDetailPage` hash anchors: `#definition` and `#runs` preserved
  as scroll targets. `#artifacts` and `#verdicts` 301-redirect to
  `/workspaces/:slug/activity?workflow=<id>&source_type=artifact` and
  `&source_type=verdict` respectively.

Object-level Ask coverage (consequence for ADR 0020 ch4):

`ChatAskButton` is currently mounted on `RunDetailPage` and
`ArtifactDetailPage` but absent on `WorkflowDetailPage` and on every
verdict / version / consumer row. This ADR makes the affordance
load-bearing — without it, leaf objects (artifact, verdict version)
have no horizontal-navigation primitive. See Adoption checklist.

## Rejected alternatives

- **Keep ADR 0019's anchored-section pattern on `WorkflowDetailPage`
  only.** Considered as a no-op. Rejected because the other three
  pages stay v0.1-shaped and ADR 0019's underlying principle — chrome
  that reflects substrate — does not carry forward without this ADR.
- **Promote Artifact and Verdict to top-level tabs (four-tab IA).**
  Considered. Rejected by ADR 0019's own rejected alternatives section
  (Artifact and Verdict are run-derived, not navigation-worthy). This
  ADR accepts that constraint and addresses it by making the detail
  page itself the surface for related-object discovery (Consumers
  section, expandable stage rows).
- **Keep `<Tabs>` IA, only re-organize tab contents.** Considered.
  Rejected because the tab structure itself smuggles the "node
  attribute = top-level surface" mental model. Re-arranging contents
  within tabs reproduces the misframing one level down.
- **Retain `IssueDetailPage` as a typed-kind specialization of
  `ArtifactDetailPage`** (different code path, same parent route).
  Considered. Rejected because parallel page paths for the same
  underlying artifact create double-entry confusion in webhook URLs,
  bookmarks, and Ask answers. The server-side `external_ref` lookup
  collapses both into a single canonical artifact URL.
- **Two ADRs (one for IA shape, one for `IssueDetailPage` retirement).**
  Considered. Rejected because the four pages share one mental model;
  splitting per-page would obscure the substrate principle and create
  redundant Context sections.

## Consequences

### Positive

- Detail pages share one IA shape. Substrate primitives map 1:1 to UI
  primitives: node ↔ row, attribute ↔ field, axis ↔ tab (where
  tabs are used at all).
- Provenance source tags become legible (ADR 0010 substrate first-class)
  because they hang on every node attribute. ADR 0022 covers their
  visual treatment.
- Artifact and Verdict, although not top-level surfaces, gain the
  navigation glue they need: `Consumers` section on artifact (the
  reverse traceability that compensates for no list view), expandable
  stage detail on RunDetail (the verdict pin point).
- One fewer page in the dashboard (`IssueDetailPage` retires); one
  fewer GitHub-coupling in the substrate-facing surface.

### Negative trade-offs

- Expanded stage rows on RunDetail can grow tall when a stage emits
  many events or runs multiple sessions. Mitigation: events table
  truncates to last 20 with `[show all]`; sessions list truncates to
  last 5.
- `ArtifactDetail`'s Content section needs a dispatch table keyed on
  `artifact_kind` (`github_issue`, `markdown`, `verdict`, `pull_request`,
  future kinds). Mitigation: ship a default render (kind label + raw
  fallback) first; per-kind branches land incrementally as needed.
- `/issues/:projectId/:number` redirect introduces a one-step lookup
  from `(projectId, number)` to `artifact_id`. Mitigation: the lookup
  is the same join that `IssueDetailPage` already performs server-side;
  no new index needed.

### Neutral

- WorkflowDetailPage hash anchors `#definition`, `#runs` preserved;
  `#artifacts`, `#verdicts` 301 to Activity-filtered URLs.
- `WorkflowEventsCard`, `WorkflowSessionsCard`, `WorkflowVerdictsTab`,
  `WorkflowArtifactsTab` components are deleted as a side effect.
- `components/sessions/SessionTable`, `SessionStateBadge`,
  `SessionLogStream` were already orphaned by ADR 0019's
  `SessionIdRedirect`; they remain dead code, cleaned up under a
  separate dead-code follow-up, not this ADR.

## Dev-process counterpart

Per ADR 0002, the dev-process analog of a detail page is a spec issue.
Spec issues already follow node-as-section: a spec has Plan (its
stages), Implementation Notes (per-stage detail), Verdict (acceptance),
Out of Scope (orthogonal). The detail page IA mirrors this shape —
a page is one document organized around nodes, not a stack of cards
organized around concerns.

The "tabs = orthogonal views of same data" rule has a dev-process
analog too: a spec's review checklist would be wrong if it used tabs
to mean "switch between unrelated content"; in practice spec sections
are anchored Markdown headings in one document. Detail pages adopt
the same shape.

## Adoption checklist

- [ ] Open implementation spec; assign issue number; backfill into
  this ADR's metadata
- [ ] Remove `<Tabs>` from `RunDetailPage`; stages timeline absorbs
  verdict, events, sessions, artifact into expanded stage detail
- [ ] Remove `Verdicts` and `Artifacts` sections from
  `WorkflowDetailPage`; add `Activity` section with link-out to
  Activity tab filtered by workflow
- [ ] Merge `ArtifactDetailPage`'s `Run Lineage`, `Version History`,
  `Vertical Lineage`, `Horizontal Lineage` Cards into one `Provenance`
  section with axis switch (Time / Origin / Relations)
- [ ] Add `Content` section to `ArtifactDetailPage` with
  `artifact_kind` dispatch; default branch + `markdown` + `github_issue`
  branches ship first
- [ ] Retire `IssueDetailPage`; add 301 redirect from
  `/issues/:projectId/:number` to `/artifacts/:id` via
  `github_issue.external_ref` server-side lookup
- [ ] Attach `ChatAskButton` to: `WorkflowDetailPage` header; each
  stage row in `RunDetailPage` expanded detail; each version row in
  `ArtifactDetailPage` Provenance Time axis; each consumer row
- [ ] Delete `WorkflowEventsCard`, `WorkflowSessionsCard`,
  `WorkflowVerdictsTab`, `WorkflowArtifactsTab`
- [ ] Preserve `#definition`, `#runs` hash anchors on
  `WorkflowDetailPage`; add `#artifacts`, `#verdicts` 301 to
  Activity-filtered URLs
- [ ] xtask lint `check-detail-page-tabs`: `pages/*DetailPage.tsx` may
  not import `<Tabs>` at the top level; tabs inside a section require
  `// tabs-allow: <reason>` marker
- [ ] Flip Status to `Accepted` once the above land

## Out of scope

- **Provenance source tag visual treatment.** Governed by ADR 0022.
- **Non-linear DAG visualization in `WorkflowDetailPage` Definition.**
  The current schema is linear stages; branching DAGs require a graph
  layout component and ship later. This ADR does not block that work
  but does not commit to it.
- **Mobile-specific behaviors.** The IA shape works at mobile width
  (stages stack vertically; expanded detail wraps). No phone-specific
  behaviors committed here.
- **Activity tab pre-filter URL schema.** The query-param schema for
  `/workspaces/:slug/activity?workflow=X&source_type=verdict` is
  governed by ADR 0019's source-type chip implementation and is not
  re-litigated here.
- **Per-`artifact_kind` Content render specifics.** This ADR commits
  to a dispatch table; the visual specification per kind ships
  incrementally as each kind earns a branch.
