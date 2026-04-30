# ADR 0002 — Process ↔ product isomorphism as design discipline

- **Status**: Accepted
- **Date**: 2026-04-19
- **Tracking issues**: #40 (architectural review)
- **Supersedes**: none
- **Superseded by**: none

## Context

Issue #40 tracked 13 sub-issues across two rounds of architectural review
and shipped through four PRs (#41, #42, #43) over the course of a few
days. Reviewing that trail revealed patterns that are *not* about the
features being shipped — they are about the development process itself
behaving like an instance of the factory we are designing.

Four concrete observations from #40's trail:

1. **Closure drift** — PR #43 landed #27, #30, #33 but its title only
   referenced #27, so #30 and #33 stayed open. This is structurally
   identical to the forge mid-tick-restart bug (#30): *coordination state
   updated outside the transaction that owns the work*. Same defect
   class, different scale.
2. **Umbrella tracker drift** — #40's Progress section required a manual
   refresh comment to reflect reality. Today this refresh is a manual
   step in `onsager-pr-lifecycle`; the same shape — *observe sub-issue
   close → propose checkbox tick → apply* — is the `ising.rule_proposed
   → safe-auto` loop (#36) we want to automate.
3. **Process lessons landing in skills** — the "multi-issue `Closes`
   discipline" lesson was captured in `onsager-pre-push`, not in code.
   Skills and CLAUDE.md currently serve as the human-readable
   supervisor policy (#37).
4. **#40 as a Refract output** — "architectural review" decomposed into
   themes, reading order, dependency graph, and acceptance cut. That is
   exactly the shape `intent.submitted → refract.decomposed` (#35) is
   supposed to produce.

Taken together, these observations point to a design principle that has
been implicit in how we work but never stated: **every Onsager primitive
already exists, informally, in how the repo is run**.

## Decision

**We adopt process ↔ product isomorphism as an explicit design
discipline**: every factory primitive ships with its dev-process
counterpart enabled, and every durable dev-process pattern is filed as
evidence for a future primitive.

Two consequences of this stance, each with teeth:

### The two-loop framing

What we have built is the **inner loop**: spec → PR → merge. The
remaining work on #40 (#35/#36/#37/#38/#39) is not five features; it is
the **outer loop** — *observe inner-loop drift → propose rule → activate
rule → modify inner loop* — sliced by time horizon:

| Horizon             | Primitive          | Current dev-process analog            |
| ------------------- | ------------------ | ------------------------------------- |
| per-intent          | Refract (#35)      | umbrella tracker (#40 itself)         |
| hours–days          | Ising (#36)        | manual umbrella tracker refresh       |
| per-decision        | Supervisor (#37)   | skills + CLAUDE.md + human sequencing |
| per-week operator   | Productivity (#38) | commented updates on the tracker      |
| per-call accounting | Budget (#39)       | `claude/*` session tokens (ungrouped) |

Every new primitive is scoped by this mapping and rejected if it cannot
be drawn.

### The same-defect-class rule

Process bugs and product bugs are the same bug at different scales. A
coordination-state-drift incident in the repo's tracking surface is not
separate from a coordination-state-drift incident in forge. Ising's rule
schema and `onsager-pre-push`'s checks should share vocabulary because
they are catching the same shape of failure. When a new process rule is
added, we ask whether it is also a latent Synodic rule; when a product
bug is found, we ask whether a dev-process instance of it has already
been observed.

## Consequences

### Positive

- New subsystems get a *pre-existing fixture* for their acceptance
  tests. #40 is Refract's fixture; the manual umbrella-tracker refresh
  step is Ising's first concrete rule waiting to be automated;
  CLAUDE.md + skills are Supervisor's initial policy corpus.
- Design conversations shift from "which feature next?" to "which loop
  is currently open?" — a sharper question that reveals dependencies
  across the #35/#36/#37/#38/#39 set.
- The dev-process surface becomes instrumentable: capturing supervisor
  decisions, tracker drift, and token usage *now* gives downstream
  subsystems real training data before they are built.

### Negative / trade-offs

- Two-bus risk. GitHub webhooks (dev-process) and `pg_notify`
  (production) are conceptually isomorphic but must stay separate at
  runtime. It is tempting to let one reach into the other; we will not.
  Isomorphism is a modeling tool, not a wiring decision.
- Over-fitting risk. #40 is *one* Refract fixture. Treating it as
  canonical would bias Refract toward architectural reviews. At least
  one non-architectural fixture (e.g. "migrate this module to async")
  must land before Refract is considered general.
- Scope-creep risk. Not every durable process pattern is a product
  primitive. The line we draw: *Synodic gates artifact transitions;
  skills govern process*. Ising proposes to Synodic only for the
  former.

### Neutral

- No code changes implied by this ADR itself — it adds design
  discipline and three template deltas (below).

## Adoption checklist

Concrete, small deltas that operationalize this ADR. Each is a single
PR-sized change.

- [ ] **ADR template** grows a "Dev-process counterpart" section: for
      every new ADR, state the analog the decision already has in how we
      run the repo. ADR 0001's analog is the GitHub webhook bus; note
      this retroactively in 0001.
- [ ] **Skill template** (`.claude/skills/*/SKILL.md`) grows a "Factory
      primitive this anticipates" section: state which Onsager
      primitive (if any) this skill is a dev-process prototype of.
- [ ] **Umbrella tracker template** grows a "Fixture-for" field at the
      top: when a tracker is opened, declare which subsystem (if any)
      will eventually consume it as acceptance data.
- [ ] **CLAUDE.md** links ADR 0002 alongside ADR 0001, with a one-line
      summary of the two-loop framing.

## Re-read of the remaining #40 sequence through this lens

Not a re-plan of the work — a re-statement of acceptance.

- **#35 Refract** — acceptance includes: given "architectural review"
  as one-line intent, Refract produces a structure comparable to #40's
  body (themes, reading order, dep graph, acceptance cut). Plus one
  non-architectural fixture.
- **#36 Ising** — launches with at least three signal kinds, including
  `umbrella_tracker_drift` and `linked_issue_not_closed` alongside
  `repeated_gate_override`. The first `ising.rule_proposed` event
  demonstrated end-to-end is "auto-close issues whose acceptance was
  met in a merged PR" — the #43 bug, automated.
- **#37 Supervisor** — precondition: emit
  `supervisor.decision_requested` / `supervisor.decision_made` events
  from the existing GitHub Actions / skill-based dev-process surfaces
  wherever a human currently intervenes. Capture before synthesize.
- **#38 Productivity** — prototype metrics against the GitHub webhook
  stream first; switch to factory events once internal volume exists.
  Validates the metric shapes before committing to dashboards.
- **#39 Budget** — track `claude/*` session tokens at tracker
  granularity so trackers like #40 can self-report unit economics
  retrospectively.

## Out of scope

- Deciding *when* each outer-loop primitive ships — that belongs in its
  own tracking issue.
- The GitHub-webhook ↔ factory-event mapping table — useful, but its
  own small doc PR.
- Any changes to subsystem scope beyond the acceptance-criteria tweaks
  noted above.
- Making skills self-modifying — that is the #37 endgame, not this ADR.
