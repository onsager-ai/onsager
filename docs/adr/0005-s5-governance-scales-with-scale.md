# ADR 0005 — S5 governance scales with scale

- **Status**: Accepted
- **Date**: 2026-05-05
- **Identity impact**: yes (establishes the meta-rule for governance evolution)
- **Tracking issues**: #249
- **Supersedes**: none
- **Superseded by**: none

## Context

A VSM (Viable System Model) read of Onsager confirms it is a viable
system in the technical sense, with most of its subsystems already
mapped:

- **S1 — operations**: agent sessions (the units that do the work).
- **S2 — coordination substrate**: the spine, including `events` /
  `events_ext`, `pg_notify`, and the workflow tables. Coordination
  happens through this substrate, not through direct calls.
- **S3 — operational management**: Forge (kernel and lifecycle),
  Stiglab (orchestration), Synodic (gates).
- **S3\* — audit channel**: Ising as the advisory continuous-improvement
  surface.

Two layers are not yet mapped:

- **S4 — outside-and-future**: there is no typed channel today that
  watches the environment and forecasts (market, ecosystem, longer
  arcs). Substance exists in scattered planning notes; structure does
  not.
- **S5 — identity**: substance exists ambient in `CLAUDE.md` (the
  seam rule, the internal-aesthetic value, the architectural-drift
  glossary) and in the four ADRs that precede this one, but it has
  never been *named* as identity. Reviewers and AI sessions infer it
  from the writing rather than reading it as such.

This ADR addresses S5. S4 stays unmapped for now.

The driving question is not whether to make S5 explicit — that's
clearly worth doing — but **how much governance machinery to stand
up around it**. Imposing nation-state-grade infrastructure (a separate
`PRINCIPLES.md`, mandatory amendment cooldowns, sunset audits, a
case-law repository of past arbitrations) at tribal scale is net cost.
Unused mechanisms decay; when they're finally needed, they aren't
reachable because nobody has been exercising them. Tribal-scale
societies operate without written constitutions and function fine.
The problems that constitutions solve begin at city-state scale.

Onsager today runs at tribal scale: 1–5 active agents, fewer than 5
arbitrations per month, one human arbiter (Marvin) in the loop on
every meaningful decision. Building governance for a scale we do not
yet operate at would generate paperwork without producing legitimacy.

## Decision

S5 governance evolves in stages, with infrastructure built **only
when scale forces it**. The current stage is tribal; subsequent stages
are sketched so that scale changes do not require greenfield
governance design.

| Stage | Signal | S5 form |
|---|---|---|
| Tribal (current) | <5 arbitrations/month, 1–5 active agents | `CLAUDE.md` identity section + ADR `Identity impact` flag + Marvin's direct arbitration |
| City-state | Recurring arbitration conflicts; different sessions giving conflicting interpretations of the same rule | Promote identity section to standalone `PRINCIPLES.md` (still amendable via regular PR) |
| Nation | Bus factor risk materializes; need to onboard collaborators who weren't present for the original decisions | Add ADR-only amendment process, sunset audit |
| Federation | Not yet planned | Not yet planned |

Triggers are **intentionally not pre-quantified**. At tribal scale,
quantitative thresholds add complexity without value — they require
metrics infrastructure that itself has to be maintained, and the
numbers chosen would be guesses against a regime we have no data for.
Real signals appear as qualitative shifts:

- "I'm starting to forget how I ruled last month."
- "Two sessions are giving conflicting interpretations of the same
  rule, and I can't tell which is right without re-deriving from
  first principles."
- "I'm spending more time arbitrating than building."

Marvin's judgment identifies these. When the upgrade signal fires, the
next stage's S5 form lands as its own ADR, building on this one.

The four identity commitments themselves (in root `CLAUDE.md` under
"What makes Onsager Onsager") are settled by the same prior discussion
that produced this ADR; they are not litigated here. This ADR is the
meta-rule about how those commitments — and the governance around them
— evolve.

## Rejected alternatives

- **Standalone `PRINCIPLES.md` now.** Considered. Rejected at current
  scale. A standalone file implies separate amendment process, which
  implies process around the process — all of which atrophies if used
  fewer than 5 times per month. The `CLAUDE.md` section captures ~80%
  of the P0 benefit (identity is named, AI sessions read it on every
  task, ADRs can flag impact) at <20% of the maintenance cost. Promote
  to standalone when the city-state signal fires.
- **Quantitative upgrade triggers** (e.g. "promote to city-state when
  >10 arbitrations/month for 3 consecutive months"). Considered.
  Rejected. Picking numbers requires data we don't have, and the
  metrics infrastructure to track them is itself overhead. Qualitative
  signals are catchable by Marvin and don't require a measurement loop.
- **Skip S5 entirely; let identity stay ambient.** Rejected. Ambient
  identity drifts under AI generation: each session re-derives from
  the surface text, and the derivations slowly diverge. Naming the
  commitments and gating ADR changes through an `Identity impact` flag
  is the minimum that arrests that drift, and it's cheap.
- **Address S4 in the same ADR.** Rejected as scope creep. S4 needs
  its own forcing function (probably the first time we mis-time a
  capability against an ecosystem shift) and its own ADR. Doing both
  in one document would dilute the S5 decision and pre-empt the S4
  one.

## Consequences

### Positive

- Governance overhead is matched to current scale. We are not paying
  for unused machinery, and the upgrade path is identified so scale
  changes do not require greenfield governance design.
- Identity content becomes explicit — readable by AI sessions on every
  task, and citable by ADRs. The 80% of P0 benefit (drift arrest,
  shared vocabulary) is captured at tribal scale.
- The `Identity impact` flag adds a small, durable signal to the ADR
  process: changes that touch identity get flagged and rationalized,
  changes that don't aren't burdened with extra ceremony.
- Future governance upgrades inherit a known starting point. The
  city-state PR is "promote the section to its own file"; the nation
  PR is "add the amendment process to the file that already exists."
  Each step is small.

### Negative / trade-offs

- Upgrade timing depends on Marvin's subjective judgment. Risk: the
  upgrade lands later than optimal because the qualitative signal is
  ambiguous or arrives slowly. The mitigation is periodic reflection
  ("am I forgetting how I ruled?") rather than a metric.
- Absence of quantitative triggers means "should have upgraded but
  didn't" is only catchable via that periodic reflection. There is no
  alarm.
- The four identity commitments concentrate authorial risk: they were
  settled in one prior discussion, and changing them carries a higher
  bar than other decisions. That's intentional — identity *should* be
  expensive to change — but it does mean a phrasing mistake is sticky.

### Neutral

- The `Identity impact` flag is the only process change. Everything
  else (regular ADR mechanics, PR review, CI lints) continues as
  before. No code change.
- ADRs 0001–0004 are not retroactively flagged. The flag applies
  going forward only.

## Dev-process counterpart

Per ADR 0002, every ADR declares the dev-process analog of the
decision it records.

This ADR's own governance weight — a single ADR plus a `CLAUDE.md`
prose section, no separate file, no amendment cooldowns — is itself
a sample of current S5 governance strength. When this ADR's form is
no longer adequate (it needs to cross-reference five related ADRs;
it grows subsections that should be their own documents; the
`Identity impact` flag starts firing on most ADRs and a coarser
mechanism is needed), that's one signal S5 governance needs to
upgrade to city-state form.

The same-defect-class rule (ADR 0002) applies: a recurring
"two sessions interpreted the rule differently" incident is the
S5 analog of a coordination-state-drift incident at the spine layer.
When the dev-process pattern accumulates, the upgrade is structural,
not procedural — exactly as it would be for a product subsystem.

## Adoption checklist

- [x] Add the "What makes Onsager Onsager" section to root `CLAUDE.md`,
      immediately before `## Architecture`.
- [x] Add a one-liner cross-link from `## Architecture` back up to the
      identity section.
- [x] Document the `Identity impact: yes/no` metadata field in
      `docs/adr/README.md` ("How to add an ADR"), and add 0005 to the
      index.
- [x] This ADR exists with `Identity impact: yes` and is linked from
      the `CLAUDE.md` identity section and the ADR index.

## Out of scope

- **A standalone `PRINCIPLES.md` file.** Explicit non-decision at
  current scale; revisit when the city-state signal fires.
- **Quantitative upgrade triggers.** Considered and rejected.
- **Retroactive `Identity impact` flagging of ADRs 0001–0004.** Going
  forward only.
- **Modifying the four identity commitments themselves.** Settled by
  the discussion that produced this ADR; phrasing-only nits welcome
  via separate PR, substance changes require their own ADR with
  `Identity impact: yes`.
- **S4 (outside-and-future) layer.** Acknowledged as missing; deferred
  to its own ADR when a forcing function appears.
