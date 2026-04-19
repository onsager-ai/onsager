# Related work: Hermes Agent

*Scope: competitive and architectural comparison for decisions that touch
Onsager positioning. Not a product review.*

## Summary

Hermes Agent (Nous Research, MIT-licensed, released Feb 2026) is an open-source
self-improving personal agent that lives on a server, hooks into 15+
messaging platforms through a single gateway, and accumulates both
episodic memory and procedural skills across sessions. Its design
philosophy — "agent as partner that grows with you" — targets individual
users running persistent assistants.

Onsager's philosophy differs at the primitive level: **factory event
bus, stigmergic coordination, artifact lifecycle, advisory feedback
loop**. The two systems solve adjacent but distinct problems; this
document pins down which boundaries are shared, which are differentiated,
and which surface as concrete backlog items.

## Architectural comparison

| Dimension            | Hermes Agent                                                     | Onsager                                                             |
| -------------------- | ---------------------------------------------------------------- | ------------------------------------------------------------------- |
| Coordination         | Synchronous `AIAgent` loop                                       | `events` / `events_ext` + `pg_notify` (ADR 0001)                    |
| Persistence unit     | Session + SQLite fact store                                      | `FactoryEvent` + `Artifact` + `Bundle` in Postgres                  |
| Memory substrate     | 4 layers: session, MEMORY.md, skills, Honcho                     | Spine as replayable event stream                                    |
| Learning             | Agent-authored skills (post-task)                                | Ising insights → Synodic rule proposals → approval                  |
| Feedback trust model | Self-evaluation (documented as flaky)                            | Advisory-only Ising + Synodic `SafeAuto`/`ReviewRequired` classes   |
| Execution layer      | 6 terminal backends (local/Docker/SSH/Daytona/Singularity/Modal) | Stiglab session orchestration + node agents                        |
| Gateway              | 15+ messaging platforms, single process                          | External consumer surface (e.g. Telegramable, outside the monorepo) |
| Target user          | Individual operator                                              | Team / factory operator                                             |
| Cost model           | Per-user OpenRouter / local                                      | Per-session `TokenUsage` on spine, aggregatable                     |
| Scheduling           | Natural-language cron                                            | Forge `SchedulingKernel` trait + `WorldState`                       |
| Intent layer         | None (user prompts directly)                                     | Refract (`Intent` → artifact tree via `Decomposer`)                 |

## Where Onsager already solves a Hermes pain point

1. **Self-evaluation unreliability.** Hermes's community-reported
   failure mode — the agent decides it succeeded when it did not, and
   that decision contaminates future skills — is exactly what Synodic's
   advisory / `ReviewRequired` split addresses. Ising observes the
   objective signal (`override_rate_by_kind`, repeated failures, stuck
   artifacts) and only crosses the `SafeAuto` threshold at 0.90
   confidence; everything else is queued for human review via
   `RuleProposalsCard`. This is a structural answer to "the agent
   cannot grade its own homework."
1. **Update instability.** Hermes's reported breakage-per-release
   cadence stems in part from runtime coupling between gateway, agent
   loop, and terminal backends. Onsager's invariant — subsystems must
   not import each other and must not be statically linked into the
   same binary — means a bad Synodic release cannot take down the
   Forge tick. ADR 0001 makes this property load-bearing rather than
   incidental.
1. **Cost blow-up.** Hermes users report per-day spend variance of two
   orders of magnitude with no factory-level aggregation. Onsager has
   `StiglabSessionCompleted.token_usage` on the spine and a `SpendCard`
   in the dashboard; the budget primitive (#39) is scoped but not yet
   first-class. See #55 for the tracking issue on elevating this.

## Non-overlap (deliberately)

- **Personal-agent UX.** Hermes's "lives where you do" narrative targets
  individual users on their own hardware. Onsager is a factory substrate,
  not a personal assistant. Do not chase platform breadth in Onsager.
- **RL trajectory export.** Hermes's training flywheel (Atropos / batch
  trajectory generation) is appropriate for a model lab. Onsager is
  infrastructure; that flywheel is not our game.
- **Messaging gateway breadth.** The 15+ platform matrix belongs outside
  the monorepo, in external consumer surfaces. Telegramable is the
  canonical example of that boundary.

## Lessons worth borrowing

1. **Progressive disclosure for skills / insights.** Hermes keeps skill
   names + descriptions in the prompt (Level 0, ~3k tokens) and loads
   full content on demand (Level 1). The same pattern applies to
   `WorldState.insights` inside the Forge kernel when the insight set
   grows beyond trivial: index in `decide()`'s input, body fetched
   lazily by analyzer.
1. **Postgres full-text search or an external recall index + LLM
   summarization for event recall.** A lightweight recall layer over
   `events_ext` — full-text index + LLM-summarized chunks — is a
   natural addition to Ising as an analyzer when the event volume
   outgrows in-memory scans.
1. **Honcho-style passive user modeling.** Not applicable at the
   factory layer, but worth considering inside a single Stiglab session
   if per-session user modeling becomes useful.

## Watch items

- **`agentskills.io` as de facto standard.** If the skill exchange
  format stabilizes, Onsager's `skills/` artifacts (and the
  `skill-evolver` outputs) should emit in a compatible schema to stay
  interoperable.
- **MCP expansion in Hermes.** Hermes ships as an MCP client;
  Onsager subsystems are not. If external code-graph or knowledge tools
  (the separated code-graph Ising, for example) expose MCP surfaces,
  they become immediately reachable from Hermes without Onsager
  shipping an integration.

## Decision log

- The code-graph analysis engine previously discussed under the name
  *Ising* is out of scope for this repo. The `ising` crate here is the
  **factory feedback engine** only. The code-graph tool is a separate
  project, not yet scoped.
- Telegramable remains outside the monorepo. If it grows, it enters
  as an external consumer of the spine, not as a subsystem.
- ADR 0001 sync-RPC retirement is parked as a parallel track behind
  the #36 feedback-loop work. New cross-subsystem request/response
  types must not land in `onsager-protocol`; see #53 for the tracking
  issue.

## References

- ADR 0001 — event-bus coordination model
- ADR 0002 — process ↔ product isomorphism
- `crates/ising/` — `repeated_failures`, `stuck_artifacts`, `gate_override`
- `crates/synodic/specs/073-feedback-ingestion-override`
- `crates/synodic/specs/076-rule-lifecycle-convergence`
- Hermes Agent docs — <https://hermes-agent.nousresearch.com/docs/>
