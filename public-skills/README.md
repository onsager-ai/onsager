# onsager-skills

Public **skills bundle** for [Onsager](https://github.com/onsager-ai/onsager) — the operating-procedures knowledge layer that pairs with portal's MCP server (the action layer). Together they are clause 1 of [ADR 0007](https://github.com/onsager-ai/onsager/blob/main/docs/adr/0007-tools-and-skills-as-the-public-contract.md): the **two-layer public contract** Onsager exposes to AI runtimes.

Tools are *what* you can call. Skills are *when to call which tool*, *how to sequence them*, and *what shapes the arguments expect*. Without the skills, an LLM staring at 11 tool descriptions has to guess the workflow; with them, the LLM sees a trigger phrase → tool sequence → example shape, and ships the right call on the first try.

> **Staging note.** This bundle is currently staged inside the main `onsager-ai/onsager` repo at `public-skills/` while the cross-repo migration to `onsager-ai/onsager-skills` is in flight. The `xtask check-tools-and-skills` lint picks it up via `ONSAGER_SKILLS_DIR=public-skills`. Once it lands in the sibling repo, this README is the canonical entry point and the staging copy goes away.

## Install

In a Claude Code session:

```bash
npx skills add onsager-ai/onsager-skills
```

This drops the four skills below into your local `~/.claude/skills/` so Claude Code triggers them automatically when their phrases match.

### Prerequisites

The skills call the portal MCP server. You need:

- An Onsager portal URL (default `https://your-org.onsager.dev`; for local dev, `http://localhost:3002`).
- A Personal Access Token with workspace access. Create one in the dashboard at **Settings → Tokens**.
- Your MCP client configured to point at `<portal-url>/mcp/messages` with the PAT in the `Authorization: Bearer <token>` header.

## The four skills

Each skill is a `SKILL.md` with YAML frontmatter — `name`, `description`, trigger-phrase list, and `allowed_tools`. The body is the operating procedure: when the trigger fires, here is the sequence of MCP tool calls to make, the argument shapes to use, and the failure modes to watch for.

| Skill | Triggers (sample) | Tools it grants |
| --- | --- | --- |
| [`onsager-design-workflow`](skills/onsager-design-workflow/SKILL.md) | "design a workflow", "create an automation", "build a pipeline" | `propose_workflow`, `edit_workflow`, `list_workflows`, `schedule_workflow` |
| [`onsager-run-workflow`](skills/onsager-run-workflow/SKILL.md) | "run this workflow", "execute the pipeline", "trigger a run" | `run_workflow`, `list_workflows`, `inspect_run` |
| [`onsager-triage-run`](skills/onsager-triage-run/SKILL.md) | "the run failed", "diagnose this", "why did it fail" | `inspect_run`, `get_stage_logs`, `propose_remediation`, `cancel_run` |
| [`onsager-explore-artifacts`](skills/onsager-explore-artifacts/SKILL.md) | "show me the artifacts", "what did this run produce" | `get_artifact`, `list_runs` |

### How the skills compose

The four skills cover one product loop:

1. **Design** a workflow (`onsager-design-workflow`).
2. **Run** it manually or wait for its trigger to fire (`onsager-run-workflow`).
3. **Explore** the artifacts a run produced (`onsager-explore-artifacts`).
4. **Triage** a run that failed or got stuck (`onsager-triage-run`).

You can hold the whole loop in one chat — the trigger phrases are designed not to collide, so "design me a workflow, run it once, and show me what it produced" cleanly chains three skills back-to-back.

## Cross-repo contract

The MCP tool registry is the single source of truth: [`crates/onsager-portal/src/mcp/registry.rs`](https://github.com/onsager-ai/onsager/blob/main/crates/onsager-portal/src/mcp/registry.rs) in the main repo. Two invariants are mechanically enforced by `xtask check-tools-and-skills`:

1. Every tool name listed in any skill's `allowed_tools` is a real, registered MCP tool.
2. Every registered MCP tool appears in `allowed_tools` of at least one skill — no orphan tools, no orphan grants.

That cross-check is exactly the [half-wired drift pattern](https://github.com/onsager-ai/onsager/blob/main/CLAUDE.md#architectural-drift-patterns-to-watch) caught for tools/skills, the way `check-events` catches producer-without-consumer for spine events.

To run the cross-check locally, with both repos checked out side-by-side:

```bash
cd onsager
ONSAGER_SKILLS_DIR=../onsager-skills cargo run -p xtask -- check-tools-and-skills
```

While the bundle is staged inside the main repo, point at the staging dir instead:

```bash
cd onsager
ONSAGER_SKILLS_DIR=public-skills cargo run -p xtask -- check-tools-and-skills
```

## License

[MIT](LICENSE)
