# ADR 0007 — Tools and skills as the public contract

- **Status**: Accepted (2026-05-11)
- **Date**: 2026-05-09
- **Identity impact**: no
- **Tracking issues**: #288 (implementation spec). Builds on ADR
  0004 (clause-1 ownership) and ADR 0006 (the dispatcher as the
  externally-reachable process); this ADR names the *protocol
  shape* of clause 1 for AI runtimes.
- **Supersedes**: none
- **Superseded by**: none

## Context

ADR 0004 named `portal` as the owner of clause 1 — the external
HTTP boundary itself. ADR 0006 named the dispatcher as the process
that is externally reachable. Both decisions answered "where does
the boundary live"; neither answered "what shape does the boundary
take for AI runtimes that want to control the factory."

The status-quo answer is REST. Portal exposes `/api/workflows`,
`/api/spine/*`, `/api/governance/*`, etc. Any AI runtime that wants
to drive the factory has to:

- Learn the REST shape from `apps/dashboard/src/lib/api/`.
- Write its own client code (HTTP, auth, error handling).
- Maintain its own knowledge of which call to make in which order
  (e.g. "to design a workflow, you POST `/api/workflows` then PATCH
  it with stages, then activate it via `workflow.activate_requested`
  on the spine").

This is a real friction. Every new runtime — Claude Code, Cursor,
Codex, Operator, custom agents — pays the integration tax. There
is no shared standard for "here is what Onsager exposes" and no
shared knowledge layer for "here is how to use it." The dashboard
chat (currently `apps/dashboard/src/components/factory/workflows/
ChatBuilder.tsx` — a placeholder with a stub LLM) sits in a
privileged position by accident: it is the only client we are
building, so it accumulates the operating procedures by default.

Two patterns from the broader AI ecosystem are converging on the
right shape:

- **MCP (Model Context Protocol)** is the emerging standard for
  exposing tools to AI runtimes. Anthropic, Cursor, OpenAI's tool
  use, and the open-source agent stacks all support MCP servers.
  A tool is a typed function (input schema, output schema, error
  model) the AI can call.
- **Skills** are the emerging standard for bundling *knowledge of
  when to call what* with the tool grants themselves. A skill is a
  markdown file with YAML frontmatter — `name`, `description`,
  trigger phrases, allowed tools — that a runtime lazy-loads when
  the trigger phrases match. Supabase ships a public skill pack
  (`npx skills add supabase/agent-skills`) alongside its MCP server;
  the same pattern works for Onsager.

Onsager already has the skills *infrastructure* in-tree — 12
internal-dev skills under `.claude/skills/` (`issue-spec`,
`ci-triage`, `dashboard-ui`, `onsager-pre-push`, etc.). What is
missing is the **outbound** layer: no MCP server, no public skills
bundle. The 2026-05-09 audit confirms the greenfield: only an empty
`crates/synodic/.mcp.json` (consumer-side, not host-side) and zero
public skills.

The user's stated direction (from the 2026-05-09 simplification
conversation) is concrete:

- Skills cover *both* knowledge and tool/API/MCP — knowledge alone
  is useless without tools; tools alone are an under-specified API.
- Tools should be exposed as a real API (MCP-based) so other AI
  runtimes can design and operate workflows without locking in to
  one runtime.
- The dashboard chat is one client among many — equivalent to
  Claude Code, Cursor, etc. — not a privileged control plane.
- HITL UI (forms, diff previews, parameter pickers) renders inside
  the chat as tool-call cards. The same tool a CLI agent calls
  pops a card in the dashboard chat for human confirmation.

This ADR commits Onsager to that shape.

## Decision

Onsager exposes itself to AI runtimes through a two-layer public
contract: an **MCP server hosted by portal** and a **public skills
bundle** distributable via `npx skills add`.

**Layer 1 — MCP server in portal.** A new module under
`crates/onsager-portal/src/mcp/` registers tools that cover both
*action* paths and *diagnostic* paths:

- *Action tools* — `propose_workflow`, `run_workflow`,
  `edit_workflow`, `schedule_workflow`, `list_workflows`,
  `list_runs`, `cancel_run`. Each tool delegates to the existing
  portal HTTP handler that owns the corresponding REST route — no
  logic duplication; the tool is a typed wrapper over an existing
  capability.
- *Diagnostic tools* — `inspect_run`, `get_stage_logs`,
  `get_artifact`, `propose_remediation`. The diagnostic surface is
  a first-class concern, not an afterthought: when a run fails the
  AI client needs structured access to logs, current state, and
  next-step suggestions, not a dead-end "something went wrong."

The MCP server reuses portal's existing `AuthUser` extractor (PAT
or session) and `require_workspace_access` helper. The dispatcher
gets one new route block — `/mcp/*` → portal — added to both
`deploy/dev/Caddyfile` and the production Caddyfile from ADR 0006.
MCP transport is HTTP (POST `/mcp/messages`); other transports
(stdio, websocket) can be added later if a use case demands.

**Layer 2 — public skills bundle.** A new artifact —
`onsager-ai/onsager-skills` — installable via `npx skills add
onsager-ai/onsager-skills`. Each skill is a `SKILL.md` with YAML
frontmatter (`name`, `description`, trigger phrases, `allowed_tools`)
plus optional reference docs and templates. Initial public skills:

- `onsager-design-workflow` — design a new workflow from intent;
  uses `propose_workflow` + `edit_workflow`.
- `onsager-run-workflow` — run a deployed workflow; uses
  `run_workflow` + `list_workflows`.
- `onsager-triage-run` — diagnose and resolve a failed run; uses
  the diagnostic tools.
- `onsager-explore-artifacts` — browse and inspect spine artifacts;
  uses `get_artifact` + REST reads where appropriate.

**Internal-dev skills stay private.** The 12 in-tree skills
(`issue-spec`, `ci-triage`, `dashboard-ui`, `onsager-pre-push`,
`onsager-pr-lifecycle`, `onsager-dev-process`, `web-testing`,
`railway`, plus the four nested per-subsystem skills) are for
working on the monorepo. They are not part of the public bundle —
publishing them would expose internal review patterns and CI
debugging procedures that have no value (and some risk) outside
the development loop.

**The dashboard chat becomes one MCP client among many.**
`ChatBuilder.tsx` evolves from a stub-LLM placeholder into an in-app
MCP client that connects to the local server. External clients
(Claude Code, Cursor, custom agents) install the skills bundle and
connect to the same MCP server. Tool surface and skill set are
identical across all clients. No operation is available in one
client and not the others.

The rule that owns this ADR:

> Every external workflow-control surface is exposed as an MCP tool
> with a typed schema. Operate-the-factory knowledge is published
> as skills. New tools and skills version together as the public
> API contract. The dashboard chat is one client; no privileged
> capability lives there that is not also reachable via MCP.

Cross-cutting alignment with prior ADRs:

- **ADR 0004** (clause 1 owned by portal): MCP server lives in
  portal, governed by the same seam-rule discipline. Tools
  internally invoke portal handlers, which emit spine events; no
  bypass of the event bus.
- **ADR 0006** (dispatcher is the external boundary): MCP traffic
  enters through the dispatcher, same as REST. Loopback to portal
  in production, inter-container in dev.
- **ADR 0001** (spine for internal coordination): MCP tools are
  *external* surface; their internal effects flow through the
  spine like every other HTTP-handler effect.

## Rejected alternatives

- **REST-only (status quo).** Forces every AI runtime to write its
  own integration layer plus maintain its own knowledge of operating
  procedures. Rejected because every new runtime pays the integration
  tax, and because the operating-procedures knowledge accumulates in
  whichever client we happen to be building (today: the dashboard
  chat). That privileged accumulation is exactly what this ADR is
  removing.
- **MCP without skills.** AI runtimes get tools but no knowledge of
  when or how to use them. Rejected because the value of the skills
  layer is recovering the human-aligned operating procedures
  (sequencing, error handling, when-to-escalate). Tools alone are an
  under-specified API surface; runtimes end up either re-deriving
  the procedures from trial-and-error or falling back to free-form
  prompting.
- **Skills without MCP.** AI runtimes get docs but have to use REST
  for actions. Rejected because skills declare tool grants, and
  without a tool standard there is no shared schema for what those
  grants mean. The combination is the value.
- **A custom protocol (Onsager-specific RPC).** Re-invents MCP.
  Rejected: MCP is the emerging standard across the AI ecosystem
  (Anthropic, Cursor, OpenAI tool use, the open-source agent
  stacks); aligning with it gets us interoperability for free.
- **Embed an AI runtime in Onsager.** Make Onsager itself the
  agent. Rejected — locks in one AI provider, conflates the factory
  (Onsager) with the operator (the AI), and runs counter to the
  user's stated direction of runtime-agnosticism. The factory
  should be operable by any MCP-supporting runtime.
- **Build the MCP server in stiglab.** Stiglab runs agent sessions;
  it is the place where AI executes *inside* Onsager. The MCP
  server is the place where AI executes *outside* Onsager. They
  are different concerns. Building MCP into stiglab also violates
  ADR 0004's clause-1 ownership — portal owns the external
  boundary, period.
- **Publish all skills (including internal-dev) as one bundle.**
  Rejected. Internal-dev skills are for working *on* the monorepo;
  they encode review patterns, CI debugging, the spec-driven
  development loop. Exposing them publicly is reach-without-reason
  and creates a maintenance burden (every internal skill change
  becomes a public-API change). Two bundles, two audiences.

## Consequences

### Positive

- **Runtime-agnostic by construction.** Any MCP-supporting AI client
  can operate Onsager: today's clients (Claude Code, Cursor,
  Codex, custom agents), and clients that don't exist yet inherit
  the surface for free.
- **Knowledge is downloadable.** New users (or new agents) install
  the skills bundle and immediately know how to design and operate
  workflows. The operating procedures stop being tribal knowledge.
- **The dashboard chat becomes structurally honest.** ChatBuilder
  evolves from a placeholder into a real MCP client; the chat
  surface is no longer privileged or special. Anything it can do,
  any client can do, and vice versa.
- **The tool surface IS the public API.** Schemas, versioning,
  deprecation — everything is explicit. No more "it depends on
  which REST endpoints you happen to know."
- **Future external integrations** (workflow control via Slack,
  Linear, GitLab events, etc.) attach to the existing MCP surface
  rather than each writing its own custom REST glue.
- **The diagnostic surface is first-class.** Failed runs surface
  structured `inspect_run` / `get_stage_logs` data instead of
  dead-ending in "something went wrong"; AI clients (and humans)
  recover the failure path the same way they navigate the success
  path.

### Negative / trade-offs

- **The tool surface is now a public contract.** Adding, removing,
  or changing a tool is a versioning event. Skills must stay
  current with tool schemas. A backwards-compatibility policy is
  needed once we have at least one tool that has evolved (deferred
  to a follow-up; until then, tools are 1.0 and changes are
  breaking).
- **Two layers to maintain.** Tools (typed) and skills (markdown)
  ship together when something changes. The dev-process counterpart
  below makes that one PR by construction, but it is still real
  per-change paperwork.
- **MCP server is new code.** Test surface, security review, and
  observability all need to be set up. Mitigation: the server
  delegates to existing portal handlers, so the new code is mostly
  protocol shell, not new business logic.
- **Onsager-the-monorepo and Onsager-the-skills-bundle are two
  release surfaces.** The bundle has to live somewhere distinct
  enough to be `npx skills add`-installable. The implementation
  spec picks the channel (subdirectory in the monorepo vs. separate
  repo) — not pre-allocated here.

### Neutral

- **REST stays.** The dashboard's non-AI calls (listing workspaces,
  fetching spine artifacts for the existing pages, etc.) keep
  using REST. MCP is additive, not a replacement.
- **ADR 0001's runtime invariant is preserved.** MCP tools are
  exposed at the seam (clause 1) and translate to spine events for
  internal coordination. No new sync HTTP between subsystems.
- **The four identity bullets are unchanged.** Event bus, artifacts,
  specs as ground truth, internal symmetry — all preserved. This
  ADR is about the *outbound* shape of the existing factory, not a
  change to its internals.

## Dev-process counterpart

Per ADR 0002, the dev-process analog: every new external
workflow-control primitive lands as a tool (with a typed schema)
plus a skill (with operating knowledge), in one PR. This mirrors
the Lever E producer/consumer/manifest rule from ADR 0004 — both
halves of the contract land together so the half-wired drift
pattern (tool-without-skill or skill-without-tool) cannot recur.

A small `xtask` check enforces it mechanically: every public tool
the MCP server registers has at least one skill in the bundle that
declares it under `allowed_tools`; every skill grant references a
real registered tool. Filed as an item in the implementation spec
opened alongside this ADR; not a blocker for the first tools to
land but a blocker for the contract to stabilize.

The product-side analog: the same shape ADR 0003 used for artifact
kinds (registry-backed, manifest-validated) extends naturally to
tool kinds. Future work may collapse the tool registry into the
existing `onsager-registry` crate so tools live alongside artifacts
and events in one place — that integration is a follow-up, not
required for this ADR to land.

## Adoption checklist

Implementation lives in the spec opened alongside this ADR
(linked under Tracking issues above). Status as of 2026-05-09:

- [ ] Build MCP server in portal: new module `crates/onsager-portal/
      src/mcp/` with server boilerplate, tool registration, request
      routing.
- [ ] Define and ship the initial action tools: `propose_workflow`,
      `run_workflow`, `edit_workflow`, `schedule_workflow`,
      `list_workflows`, `list_runs`, `cancel_run`. Each delegates
      to the corresponding existing portal HTTP handler.
- [ ] Define and ship the initial diagnostic tools: `inspect_run`,
      `get_stage_logs`, `get_artifact`, `propose_remediation`.
- [ ] Add `/mcp/*` route block to `deploy/dev/Caddyfile` and the
      production Caddyfile from ADR 0006.
- [ ] Create the public skills bundle (channel TBD by the
      implementation spec). Initial skills: `onsager-design-workflow`,
      `onsager-run-workflow`, `onsager-triage-run`,
      `onsager-explore-artifacts`.
- [ ] Migrate `apps/dashboard/src/components/factory/workflows/
      ChatBuilder.tsx` to be an MCP client of the local server.
      Render tool calls as inline HITL UI cards.
- [ ] `xtask check-tools-and-skills` consistency lint: every public
      tool has at least one skill granting it; every skill grant
      references a real tool.
- [ ] Update root `CLAUDE.md` with the MCP + skills surface; cross-
      link from the seam-rule section.
- [ ] Flip ADR 0007 to `Accepted` in the same PR that lands the
      first end-to-end tool + skill pair (likely
      `onsager-design-workflow` ↔ `propose_workflow`).

## Out of scope

- **Replacing REST with MCP.** REST stays for the dashboard's non-AI
  calls. MCP is additive. A future ADR may revisit consolidation
  once the tool surface has matured.
- **Onsager hosting AI runtimes.** Stiglab continues to run agent
  sessions inside the factory; that is unrelated to MCP, which is
  about how the factory is *consumed* by external runtimes.
- **MCP server authentication.** Reuses portal's existing
  `AuthUser` extractor (PAT or session). No new auth mechanism.
- **Tool-call rate limiting and quotas.** Future concern; reuses
  portal's existing rate-limit infrastructure when added.
- **Multi-tenant skill bundles** (per-workspace custom skills).
  Future concern. This ADR commits to one public bundle.
- **Backwards-compatibility policy for the tool surface.**
  Deferred until at least one tool has evolved. Until then, tools
  are 1.0 and changes are breaking; the implementation spec
  documents this for the initial release.
- **Migrating the existing in-tree internal-dev skills to a
  shared skills standard.** They are already in skills format
  (`.claude/skills/`); the question of unifying them with the
  public bundle is deferred and may never make sense (they are
  audience-distinct).
