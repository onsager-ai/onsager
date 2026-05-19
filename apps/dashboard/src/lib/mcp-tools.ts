// Typed view over the portal MCP tool registry. The Rust registry at
// `crates/onsager-portal/src/mcp/registry.rs` is the source of truth for
// what tools exist and which `ToolCategory` they belong to. This file
// mirrors that registry for the dashboard side: it pairs each tool name
// with the HitlCard slot the chat renderer should use, plus a small
// amount of UI metadata (commit-button label, side-effect copy,
// type-to-confirm prompts) that does not live on the wire.
//
// Cross-checked by `xtask check-hitl-coverage`: every mutation tool in
// the Rust registry must appear here with a non-`read_only` slot, and
// every read-only tool must appear here as `read_only`. Drift is a hard
// CI failure.
//
// Hand-typed for v1. The schemars-derived Rust-as-SSOT TS codegen is
// filed as a follow-up; once that pipeline lands the entries below
// collapse to the slot + UI metadata.

import type { HitlCard, HitlFieldSpec } from "@/components/chat/hitl-types"

/** Mirror of `ToolCategory` from `crates/onsager-portal/src/mcp/registry.rs`. */
export type ToolSlot = "constructive" | "diff" | "destructive" | "read_only"

/**
 * One tool the dashboard knows how to render. `category` decides whether
 * a tool call surfaces as a HitlCard (mutation) or a plain info block
 * (read-only). For mutation tools, `buildCard` turns the agent's
 * proposed arguments into a HitlCard config the primitive can render.
 */
export interface McpToolBinding {
  name: string
  category: ToolSlot
  /** Short label used as the card title / info-block heading. */
  title: (args: Record<string, unknown>) => string
  /**
   * Build the card config for a mutation tool. `undefined` for read-only
   * tools — those render via `renderInfo` instead.
   */
  buildCard?: (args: Record<string, unknown>) => HitlCard
  /**
   * Build a plain text summary for read-only tools. Used in the chat
   * stream as a non-card info block.
   */
  renderInfo?: (args: Record<string, unknown>) => string
}

// -----------------------------------------------------------------------------
// Field helpers
// -----------------------------------------------------------------------------

function str(args: Record<string, unknown>, key: string, fallback = ""): string {
  const v = args[key]
  return typeof v === "string" ? v : fallback
}

function bool(args: Record<string, unknown>, key: string): boolean | undefined {
  const v = args[key]
  return typeof v === "boolean" ? v : undefined
}

function stagesSummary(args: Record<string, unknown>): string {
  const s = args.stages
  if (!Array.isArray(s)) return "no stages"
  return `${s.length} stage${s.length === 1 ? "" : "s"}`
}

function field(
  label: string,
  value: unknown,
  opts: Partial<HitlFieldSpec> = {},
): HitlFieldSpec {
  return {
    label,
    value: value == null ? "" : String(value),
    editable: opts.editable ?? false,
    ...opts,
  }
}

// -----------------------------------------------------------------------------
// Tool bindings — one entry per Rust registry tool
// -----------------------------------------------------------------------------

const propose_workflow: McpToolBinding = {
  name: "propose_workflow",
  category: "constructive",
  title: (args) => `Create workflow${str(args, "name") ? ` · ${str(args, "name")}` : ""}`,
  buildCard: (args) => {
    const trigger = args.trigger as Record<string, unknown> | undefined
    const triggerKind =
      trigger && typeof trigger.kind === "string" ? trigger.kind : "(none)"
    const installId =
      typeof args.install_id === "number" || typeof args.install_id === "string"
        ? String(args.install_id)
        : "0"
    return {
      kind: "constructive",
      title: `Create workflow${str(args, "name") ? ` · ${str(args, "name")}` : ""}`,
      summary: stagesSummary(args),
      body: {
        fields: [
          field("Name", str(args, "name"), { editable: true, key: "name" }),
          field("Workspace", str(args, "workspace_id")),
          field("Trigger", triggerKind),
          field("Install id", installId),
          field("Stages", stagesSummary(args)),
        ],
      },
      commit: { label: "Create workflow", intent: "primary" },
      reject: { label: "Reject" },
    }
  },
}

const propose_workflow_draft: McpToolBinding = {
  name: "propose_workflow_draft",
  category: "constructive",
  title: (args) =>
    `Propose draft${str(args, "name") ? ` · ${str(args, "name")}` : ""}`,
  buildCard: (args) => {
    const trigger = args.trigger as Record<string, unknown> | undefined
    const label =
      trigger && typeof trigger.label === "string" ? trigger.label : "(unset)"
    return {
      kind: "constructive",
      title: `Propose workflow draft${str(args, "name") ? ` · ${str(args, "name")}` : ""}`,
      summary: stagesSummary(args),
      body: {
        fields: [
          field("Name", str(args, "name"), { editable: true, key: "name" }),
          field("Trigger label", label),
          field("Stages", stagesSummary(args)),
        ],
      },
      commit: { label: "Save draft", intent: "primary" },
      reject: { label: "Reject" },
    }
  },
}

const run_workflow: McpToolBinding = {
  name: "run_workflow",
  category: "destructive",
  title: (args) => `Run workflow · ${str(args, "workflow_id", "unknown")}`,
  buildCard: (args) => ({
    kind: "destructive",
    title: `Run workflow · ${str(args, "workflow_id", "unknown")}`,
    body: {
      info: `Fires the workflow's manual trigger \`${str(args, "trigger_name", "manual")}\` and starts a new run.`,
    },
    sideEffects: [
      "Emits `trigger.fired` on the workflow's trigger stream",
      "Forge picks up the trigger and starts a new run for the artifact",
    ],
    reversibility: "Reversible — cancel the run from its detail page if needed.",
    commit: { label: "Run workflow", intent: "destructive" },
    reject: { label: "Don't run" },
  }),
}

const edit_workflow: McpToolBinding = {
  name: "edit_workflow",
  category: "diff",
  title: (args) => `Edit workflow · ${str(args, "workflow_id", "unknown")}`,
  buildCard: (args) => {
    const before: Record<string, string> = {}
    const after: Record<string, string> = {}
    const active = bool(args, "active")
    if (active !== undefined) {
      before.active = "(current value)"
      after.active = active ? "true" : "false"
    }
    if (typeof args.name === "string") {
      before.name = "(current value)"
      after.name = String(args.name)
    }
    return {
      kind: "diff",
      title: `Edit workflow · ${str(args, "workflow_id", "unknown")}`,
      summary: `${Object.keys(after).length} field${Object.keys(after).length === 1 ? "" : "s"} modified`,
      body: { before, after },
      commit: { label: "Apply changes", intent: "primary" },
      reject: { label: "Discard" },
    }
  },
}

const schedule_workflow: McpToolBinding = {
  name: "schedule_workflow",
  category: "diff",
  title: (args) => `Schedule workflow · ${str(args, "workflow_id", "unknown")}`,
  buildCard: (args) => {
    const trigger = args.trigger as Record<string, unknown> | undefined
    const kind = trigger && typeof trigger.kind === "string" ? trigger.kind : "(unknown)"
    return {
      kind: "diff",
      title: `Schedule workflow · ${str(args, "workflow_id", "unknown")}`,
      summary: `Trigger → ${kind}`,
      body: {
        before: { trigger: "(current trigger)" },
        after: { trigger: kind },
      },
      commit: { label: "Update schedule", intent: "primary" },
      reject: { label: "Discard" },
    }
  },
}

const list_workflows: McpToolBinding = {
  name: "list_workflows",
  category: "read_only",
  title: () => "List workflows",
  renderInfo: (args) => {
    const ws = str(args, "workspace_id")
    return ws ? `Listing workflows in workspace ${ws}.` : "Listing workflows."
  },
}

const list_runs: McpToolBinding = {
  name: "list_runs",
  category: "read_only",
  title: () => "List runs",
  renderInfo: (args) =>
    `Listing recent runs for workflow ${str(args, "workflow_id", "(unknown)")}.`,
}

const cancel_run: McpToolBinding = {
  name: "cancel_run",
  category: "destructive",
  title: (args) => `Cancel run · ${str(args, "artifact_id", "unknown")}`,
  buildCard: (args) => ({
    kind: "destructive",
    title: `Cancel run · ${str(args, "artifact_id", "unknown")}`,
    body: {
      info: "Aborts the in-flight run and archives the artifact.",
    },
    sideEffects: [
      "Sets `artifacts.state = 'archived'`",
      "Emits `artifact.archived` on the `forge:<artifact_id>` stream",
      "In-flight stage work is dropped — downstream consumers see the abort",
    ],
    reversibility:
      "Irreversible at the artifact level — the row is archived synchronously. Re-runs are a new artifact.",
    confirmTyping: {
      promptLabel: "Type the artifact id to confirm",
      expectedValue: str(args, "artifact_id"),
    },
    commit: { label: `Cancel run ${str(args, "artifact_id", "")}`.trim(), intent: "destructive" },
    reject: { label: "Keep running" },
  }),
}

const inspect_run: McpToolBinding = {
  name: "inspect_run",
  category: "read_only",
  title: () => "Inspect run",
  renderInfo: (args) =>
    `Inspecting run ${str(args, "artifact_id", "(unknown)")}.`,
}

const get_stage_logs: McpToolBinding = {
  name: "get_stage_logs",
  category: "read_only",
  title: () => "Stage logs",
  renderInfo: (args) =>
    `Fetching stage logs for session ${str(args, "session_id", "(unknown)")}.`,
}

const get_artifact: McpToolBinding = {
  name: "get_artifact",
  category: "read_only",
  title: () => "Get artifact",
  renderInfo: (args) =>
    `Fetching artifact ${str(args, "artifact_id", "(unknown)")}.`,
}

const propose_remediation: McpToolBinding = {
  name: "propose_remediation",
  category: "read_only",
  title: () => "Propose remediation",
  renderInfo: (args) =>
    `Reading failure pointers for run ${str(args, "artifact_id", "(unknown)")}.`,
}

// -----------------------------------------------------------------------------
// Registry
// -----------------------------------------------------------------------------

const BINDINGS: McpToolBinding[] = [
  propose_workflow,
  propose_workflow_draft,
  run_workflow,
  edit_workflow,
  schedule_workflow,
  list_workflows,
  list_runs,
  cancel_run,
  inspect_run,
  get_stage_logs,
  get_artifact,
  propose_remediation,
]

/** All known MCP tools, in registry order. */
export function mcpToolBindings(): readonly McpToolBinding[] {
  return BINDINGS
}

/** Look up a binding by tool name. Returns `undefined` for unknown tools. */
export function findMcpTool(name: string): McpToolBinding | undefined {
  return BINDINGS.find((t) => t.name === name)
}

/** Is this tool a mutation? Mutations route through a HitlCard. */
export function isMutationTool(name: string): boolean {
  const b = findMcpTool(name)
  return b !== undefined && b.category !== "read_only"
}
