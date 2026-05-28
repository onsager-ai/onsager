# ADR 0022 — Provenance source tag as first-class UI

- **Status**: Proposed
- **Date**: 2026-05-27
- **Identity impact**: no. Refines how ADR 0010's substrate invariant surfaces in UI; does not change the invariant itself.
- **Tracking issues**: #488 (implementation spec). Depends on ADR 0021's implementation per #487 for the detail-page surfaces this ADR attaches to.
- **Supersedes**: none
- **Superseded by**: none

## Context

ADR 0010 promoted provenance to a substrate first-class concept with two flavors — `Deterministic` (output is verifiable from inputs without external state) and `Uncertain` (output depends on a non-deterministic executor: LLM, network, human, wall-clock) — each carrying a `SourceTag` payload that identifies the specific contributor.

In the dashboard today this concept has zero UI surface. Examples from current code:

- `pages/ArtifactDetailPage.tsx:340-395` — `Version History` table columns are `Version / Session / Recorded`. No source column. An operator inspecting a version cannot tell whether it came from a deterministic forge step or from an agent self-report without drilling into the session.
- `pages/RunDetailPage.tsx:254-258` — verdict and event tabs render status and timestamp; the verdict's `source` field is dropped on the way to the row.
- `pages/ActivityPage.tsx:23-79` — source-type chips exist (Artifact, Run, Verdict, Trigger, Issue) but they reflect the *originating subsystem*, not the Det/Unc flavor of each row. An operator filtering for "Verdict" sees both deterministic CI verdicts and agent self-reports as one flat list.
- `pages/WorkflowDetailPage.tsx:225-244` — the Runs section shows completed run status (pass/fail) without the run's terminal-verdict source. Two runs that look identical in the list can have very different epistemic standing.

The substrate's first-class concept has no UI vocabulary. ADR 0010's investment in tagging every node attribute with provenance metadata is not reaching the operator.

ADR 0021's per-detail-page IA reorganization places node attributes (verdict, events, sessions, output) into expanded detail rows on their owning node. Each of those rows is a candidate site for `SourceTag` rendering. Without a uniform UI vocabulary for the tag, the new IA shape inherits the same provenance-blindness as the old.

## Decision

Adopt a uniform UI vocabulary for provenance source tags. Apply it to every surface where Det/Unc attribution is meaningful.

### Vocabulary

|Flavor          |Treatment                                                              |Tight contexts |
|----------------|-----------------------------------------------------------------------|---------------|
|`Deterministic` |Mono small-caps, ink-tone neutral, tracked: `DETERMINISTIC`            |`DET`          |
|`Uncertain`     |Italic serif, accent-tone, dagger superscript: *uncertain*<sup>†</sup> |*unc*<sup>†</sup>|

Rationale for the typographic split:

- The difference between "verifiable from inputs" and "self-reported by a non-deterministic executor" is *epistemic*, not categorical. Color-coded chips reduce the distinction to status (green vs amber → "good vs warning"), which is wrong: an uncertain verdict can be correct and a deterministic verdict can be wrong about its inputs.
- The dagger (†) is the academic footnote convention for "this claim requires verification". Readers familiar with the convention recognize it; readers unfamiliar see a typographic difference and reach for the tooltip.
- Mono small-caps is the typographic equivalent of a printed stamp/seal: settled, neutral, official. Italic serif is the voice of editorial qualification: "as reported", "in the author's view".
- The typographic treatment uses no extra horizontal space beyond the label string itself, important on row-dense surfaces.

### Surfaces required to render `SourceTag`

1. **`RunDetailPage` expanded stages** — each event row, each verdict, each session output (per ADR 0021)
1. **`ArtifactDetailPage` Provenance section** — every node in the graph, regardless of which axis is active (Time / Origin / Relations, per ADR 0021)
1. **`ArtifactDetailPage` Consumers section** — each consumer row carries the SourceTag of the run-stage that consumed it
1. **`ActivityPage` rows** — each event row in the stream
1. **`WorkflowDetailPage` Activity section** — each recent-run row carries the source of that run's terminal verdict
1. **Detail-page header strips** — when the page's primary object is itself produced by an uncertain executor (e.g., agent-produced artifact), the tag appears beside the producer link in the header

### Tooltip

Every `SourceTag` carries a hover/focus tooltip:

- Deterministic: *"Source: Deterministic. Output is verifiable from inputs without external state."*
- Uncertain: *"Source: Uncertain. Output depends on a non-deterministic executor (LLM, network, human, wall-clock). Verify before relying on it."*

### Filter dimension

`SourceTag` becomes a filter axis parallel to existing ones:

- Activity tab gains a `source` chip group (`All` / `Det` / `Unc`) alongside the existing `source-type` chips
- Workflows tab gains a `last verdict source` per-row pin; filterable via a chip on the tab header

### Component contract

A shared `<SourceTag kind="deterministic" | "uncertain" />` component lands in `components/ui/source-tag.tsx`. The component:

- Defaults to long-form label
- Accepts `compact={true}` for tight contexts (table cells under 100px wide)
- Renders tooltip via the existing `<Tooltip>` primitive
- Has no `color` or `variant` prop — typography is the visual axis, not theme

## Rejected alternatives

- **Color-coded badge chips** (e.g., emerald pill for deterministic, amber pill for uncertain). Considered as the obvious option. Rejected because (a) colored chips conflate Det/Unc with status semantics (pass/warn), which they are not — an uncertain output can be a passing verdict; a deterministic output can be a failed one; (b) accessibility: colorblind operators lose the entire signal, and no shape-based fallback maps cleanly onto a chip; (c) chip noise on dense rows — a 20-row events list with amber chips on every row reads as "everything is warning" and dilutes both status chips and source chips into the same visual register.
- **Single-icon variant** (lock for Det, question mark for Unc). Considered. Rejected because icons compete with status icons (check, cross, spinner) for the same visual register; two icon families on each row crowds the start-of-row gutter where status already lives.
- **Render only on hover/focus** (the row shows status; source tag appears on disclosure). Considered as a density-reduction option. Rejected because operators scanning a verdict list need source flavor at a glance — hidden-until-hover defeats the substrate's first-class status. ADR 0010 made provenance first-class so it could shape decisions; hiding it from default rendering reverses that decision in the surface where it matters.
- **Selective rendering** (render only on `RunDetail` and Provenance; omit elsewhere). Considered. Rejected because the substrate doesn't have a notion of "key" surfaces — every node carries a source tag at the data level. Selective UI rendering creates the impression that some events are unattributed, which is false.
- **Inline prefix-symbol** (e.g., a small ◆ before deterministic, ◇ before uncertain rows). Considered as a minimal alternative. Rejected because the symbols carry no semantic intuition and need the same tooltip explanation as text labels — without buying any density advantage over the typographic-label form.

## Consequences

### Positive

- Operators see `SourceTag` on every surface where a substrate node attribute appears. ADR 0010's first-class invariant gains UI parity.
- A new filter axis (Det/Unc) opens for triage: "show me only uncertain verdicts from this workflow" becomes a single chip selection rather than a manual scan.
- The typographic treatment costs no extra horizontal space beyond the label string, important on dense surfaces (Activity stream, event tables).
- The Det/Unc distinction visibly attaches to the executor model: a `claude-haiku-4.5` agent stage's verdict reads as *uncertain*<sup>†</sup>, a CI verdict reads as `DETERMINISTIC` — and the operator can use the distinction to calibrate trust without leaving the row.

### Negative trade-offs

- Surfaces with very high row counts (Activity stream paginated to 100+ rows) get a new visual element on every row. Mitigation: the `compact={true}` form (`DET` / *unc*<sup>†</sup>) is permitted in contexts under 100px column width and on the Activity stream by default.
- Operators unfamiliar with the dagger convention see an unfamiliar symbol and need the tooltip on first encounter. Mitigation: tooltip on every instance; one-time onboarding hint on first verdict drill-in (FTUE event `ftue.source_tag.first_view`).
- The italic serif typeface (Spectral or equivalent) must be available on every surface that renders `SourceTag` — including surfaces that don't currently load editorial type (e.g., embedded preview thumbnails, OG-image generators). Mitigation: the `<SourceTag>` component declares its own font dependency at the bundle level, so any consuming surface inherits it transitively.

### Neutral

- ADR 0010's underlying schema is unchanged. No new substrate API, no new event field. `source` is already on every event, verdict, and version record.
- The component lives in `components/ui/`, not in `components/governance/` or `components/observers/`, signalling that it is a shared UI primitive, not a subsystem widget.
- The `xtask check-source-tag-coverage` lint (Adoption checklist) is the dev-process enforcement counterpart of ADR 0010's substrate invariant — same mechanism, different layer.

## Dev-process counterpart

Per ADR 0002, the dev-process analog is the spec's acceptance attribution. A spec's acceptance can be *verifiable* (CI pass + schema match) or *self-reported* (author confidence; reviewer trust). Treating both with the same dev-process weight is the same mistake the dashboard makes by rendering all provenance attributions identically.

The corresponding spec-authoring convention: every acceptance line in a spec's "Done when" section is implicitly tagged. When ambiguous, authors annotate explicitly: *"CI passes (Deterministic)"* / *"reviewer judges output correct (Uncertain)"*. This matches the UI convention adopted here: typographic difference where the epistemic weight differs.

## Adoption checklist

- [ ] Open implementation spec; assign issue number; backfill into this ADR's metadata
- [ ] Land `components/ui/source-tag.tsx` with `kind` and `compact` props, tooltip integration, no color/variant props
- [ ] Mount `<SourceTag>` on the six surfaces enumerated in the Decision section
- [ ] Add `source` chip group (`All` / `Det` / `Unc`) to `ActivityPage` alongside existing source-type chips
- [ ] Add `last verdict source` per-row indicator + tab-header filter to `WorkflowsPage`
- [ ] Tooltip copy reviewed by ADR 0010 author for substrate-fidelity
- [ ] FTUE event `ftue.source_tag.first_view` instrumented; one-time hint shipped on first verdict drill-in
- [ ] xtask lint `check-source-tag-coverage`: detail-page row components rendering a substrate node attribute must reference `<SourceTag>`; escape comments `// source-tag-omit: <reason>` allowed
- [ ] Flip Status to `Accepted` once the above land

## Out of scope

- **Detail page IA shape.** Governed by ADR 0021. This ADR depends on the surfaces that ADR 0021 introduces but does not specify their structure.
- **Chat (ADR 0020) integration.** Whether scope-local Ask buttons' answer rendering surfaces `SourceTag` in their responses is governed by chat IA + chat persona prompt design, not this ADR.
- **Backend Det/Unc classification heuristics.** How an event is labeled at write time is governed by ADR 0010 plus per-executor specs. This ADR consumes the labels; it does not produce them.
- **Dark-theme treatment.** Visual specification covers light theme only. Dark-theme color and contrast targets land when dark-theme support lands.
- **Localization of dagger convention.** The dagger is a Western scholarly typographic convention; locales where it carries a different signal would need a per-locale override. Out of scope for this ADR — revisit when localization scope opens.
