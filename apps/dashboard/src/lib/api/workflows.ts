import { ApiError, request } from "./client";
import type {
  GovernanceEvent,
  SpineArtifact,
  Workflow,
  WorkflowStage,
  WorkflowGateKind,
  CreateWorkflowRequest,
  CreateWorkflowStage,
  RunDetail,
  RunLinkedSession,
} from "./types";
import type { WorkflowRun } from "./generated/WorkflowRun";
import type { StageRunStatus } from "./generated/StageRunStatus";
// Generated wire shapes (spec #434 cascaded ts-rs into `onsager-spine` so
// `TriggerKind` and `Workflow` are emitted from Rust). The dashboard
// keeps a richer nested UI-side `Workflow` (with `status`, flattened
// `trigger`); these adapters translate generated → UI shape so the
// rest of the app doesn't have to know the wire format.
import type { Workflow as BackendWorkflow } from "./generated/Workflow";
import type { TriggerKind as BackendTrigger } from "./generated/TriggerKind";

interface BackendWorkflowStage {
  id: string;
  workflow_id: string;
  seq: number;
  gate_kind: WorkflowGateKind;
  params: Record<string, unknown>;
}

// Pack a UI stage into the backend's `{ gate_kind, params }` pair. The
// UI-only `name` rides in `params` so it survives the round-trip without a
// backend-schema change.
export function stageToCreateStage(s: WorkflowStage): CreateWorkflowStage {
  return {
    gate_kind: s.gate_kind,
    params: {
      ...(s.config ?? {}),
      name: s.name,
    },
  };
}

function stageFromBackend(s: BackendWorkflowStage): WorkflowStage {
  const params = (s.params ?? {}) as Record<string, unknown>;
  const name = typeof params.name === "string" ? params.name : undefined;
  // Strip the UI-only display fields. `artifact_kind` is a legacy field
  // (demoted in #568) dropped on read so it never re-serializes from old
  // persisted workflows.
  const {
    name: _n,
    artifact_kind: _a,
    ...config
  } = params as Record<string, unknown>;
  void _n;
  void _a;
  return {
    id: s.id,
    name: name ?? defaultStageName(s.gate_kind),
    gate_kind: s.gate_kind,
    config,
  };
}

function workflowFromBackend(
  w: BackendWorkflow,
  stages: BackendWorkflowStage[] = [],
): Workflow {
  // Per-kind UI translation lives here so the rest of the app can keep
  // its richer nested trigger shape. The registry
  // (`/api/registry/triggers`) is the source of truth for which kinds
  // exist; the discriminated `TriggerKind` union narrows by `kind`.
  const { repo, label, manualName } = extractTriggerFields(w.trigger);
  const [repoOwner = "", repoName = ""] = repo.split("/");
  return {
    id: w.id,
    workspace_id: w.workspace_id,
    name: w.name,
    preset: w.preset_id,
    status: w.active ? "active" : "draft",
    // ADR 0024 lifecycle axes (#512), carried straight from the wire so
    // the detail page + run button gate on `readiness` rather than
    // re-deriving from `active`. Narrow the generated `string` to the UI
    // union, defaulting to the *safe* side: only an exact `"ready"`
    // enables ready-gated affordances (e.g. the run button), so an
    // unknown/new backend state never silently unlocks them.
    readiness: w.readiness === "ready" ? "ready" : "drafted",
    autofire:
      w.autofire === "active" ? "active" : w.autofire === "paused" ? "paused" : "off",
    trigger: {
      kind: "github-label",
      // `install_id` is a nullable token-mint cache (ADR 0024); null means
      // "no cached install — resolved live at activation". The UI's
      // no-install convention is the empty string, not the literal "null".
      install_id: w.install_id == null ? "" : String(w.install_id),
      repo_owner: repoOwner,
      repo_name: repoName,
      label,
      kind_tag: w.trigger.kind,
      manual_name: manualName,
    },
    stages: stages.map(stageFromBackend),
    created_at: w.created_at,
    updated_at: w.updated_at,
  };
}

// Exhaustive over every `TriggerKind` variant — when a new trigger
// kind lands in `onsager-spine::TriggerKind`, ts-rs widens this union
// and the `_exhaustive: never` assignment below fails to compile,
// forcing this function to decide how the new variant maps onto the
// UI's flat `WorkflowTrigger` shape (instead of silently rendering
// with blank fields).
function extractTriggerFields(t: BackendTrigger): {
  repo: string;
  label: string;
  manualName: string;
} {
  switch (t.kind) {
    case "github_issue_webhook":
      return { repo: t.repo, label: t.label, manualName: "" };
    case "github_pull_request_closed":
    case "github_workflow_run_completed":
      return { repo: t.repo, label: "", manualName: "" };
    case "manual":
      return { repo: "", label: "", manualName: t.name };
    case "telegram_webhook":
    case "cron":
    case "delay":
    case "interval":
    case "spine_event":
    case "pg_notify":
    case "outbox_row":
    case "replay":
      return { repo: "", label: "", manualName: "" };
    default: {
      const _exhaustive: never = t;
      void _exhaustive;
      return { repo: "", label: "", manualName: "" };
    }
  }
}

function defaultStageName(gate: WorkflowGateKind): string {
  switch (gate) {
    case "agent-session":
      return "Agent session";
    case "external-check":
      return "CI check";
    case "governance":
      return "Governance";
    case "manual-approval":
      return "Manual approval";
  }
}

/// The two orthogonal filter axes on the Workflows tab (ADR 0024 / spec
/// #509). `"all"` means the axis is unconstrained; the dashboard sends
/// `?readiness=<v>&autofire=<v>` (each omitted when `"all"`), and the
/// backend ANDs the two together.
export type ReadinessFilter = "all" | "drafted" | "ready";
export type AutofireFilter = "all" | "off" | "active" | "paused";

/// Per-axis-value counts for the filter strip. Each axis reports its own
/// totals so the two filter groups show independent counts. Mirrors the
/// portal `workflow_axis_counts` response.
export interface WorkflowAxisCounts {
  all: number;
  readiness: {
    drafted: number;
    ready: number;
  };
  autofire: {
    off: number;
    active: number;
    paused: number;
  };
}

export interface ListWorkflowsResult {
  workflows: Workflow[];
  counts: WorkflowAxisCounts;
}

const EMPTY_AXIS_COUNTS: WorkflowAxisCounts = {
  all: 0,
  readiness: { drafted: 0, ready: 0 },
  autofire: { off: 0, active: 0, paused: 0 },
};

export const workflows = {
  // Workflows (issue #82) — CRUD + live runs. The dashboard is the only
  // client today; these wrappers translate backend → UI shape.
  listWorkflows: async (
    workspaceId: string,
    filter?: { readiness?: ReadinessFilter; autofire?: AutofireFilter },
  ): Promise<ListWorkflowsResult> => {
    if (!workspaceId) throw new ApiError("workspaceId is required", 400);
    const query = new URLSearchParams({ workspace_id: workspaceId });
    if (filter?.readiness && filter.readiness !== "all") {
      query.set("readiness", filter.readiness);
    }
    if (filter?.autofire && filter.autofire !== "all") {
      query.set("autofire", filter.autofire);
    }
    const raw = await request<{
      workflows: BackendWorkflow[];
      counts?: WorkflowAxisCounts;
    }>(`/workflows?${query.toString()}`);
    return {
      workflows: raw.workflows.map((w) => workflowFromBackend(w)),
      counts: raw.counts ?? EMPTY_AXIS_COUNTS,
    };
  },
  // Fan-out across every workspace the user belongs to. Stiglab's list
  // endpoint is workspace-scoped; cross-workspace "do I have any workflows
  // yet?" queries (empty-state gates, first-run redirect) need this shape.
  // We hit `/workspaces` once and one `/workflows?workspace_id=…` per
  // workspace; fine for the workspace counts we target (typically 1–3).
  listWorkflowsForUser: async (): Promise<{ workflows: Workflow[] }> => {
    const { workspaces } = await request<{ workspaces: { id: string }[] }>(
      "/workspaces",
    );
    const lists = await Promise.all(
      workspaces.map((w) =>
        request<{ workflows: BackendWorkflow[] }>(
          `/workflows?workspace_id=${encodeURIComponent(w.id)}`,
        ).then((r) => r.workflows.map((wf) => workflowFromBackend(wf))),
      ),
    );
    return { workflows: lists.flat() };
  },
  // Workspace-scoped cross-workflow runs list (#454 / ADR 0019). Backs
  // the dashboard's Runs tab. Optional filters mirror the portal's
  // `WorkspaceRunsQuery` (#464): `workflowId` narrows to a single
  // workflow, `status` filters at the SQL layer, `since` / `until` are
  // ISO-8601 bounds on `updated_at`.
  listWorkspaceRuns: (
    workspaceId: string,
    opts?: {
      workflowId?: string;
      status?: StageRunStatus;
      since?: string;
      until?: string;
      limit?: number;
    },
  ): Promise<{ runs: WorkflowRun[] }> => {
    if (!workspaceId) throw new ApiError("workspaceId is required", 400);
    const params = new URLSearchParams();
    if (opts?.workflowId) params.set("workflow_id", opts.workflowId);
    if (opts?.status) params.set("status", opts.status);
    if (opts?.since) params.set("since", opts.since);
    if (opts?.until) params.set("until", opts.until);
    if (opts?.limit) params.set("limit", String(opts.limit));
    const raw = params.toString();
    // `qs` is the lint-recognized variable name for query suffixes
    // in the api-contract matcher; leading `?` lives here so the
    // template stays a simple `${qs}` interpolation.
    const qs = raw ? `?${raw}` : "";
    return request<{ runs: WorkflowRun[] }>(
      `/workspaces/${encodeURIComponent(workspaceId)}/runs${qs}`,
    );
  },
  getWorkflow: async (id: string): Promise<{ workflow: Workflow }> => {
    const raw = await request<{
      workflow: BackendWorkflow;
      stages: BackendWorkflowStage[];
    }>(`/workflows/${encodeURIComponent(id)}`);
    return { workflow: workflowFromBackend(raw.workflow, raw.stages) };
  },
  createWorkflow: async (
    body: CreateWorkflowRequest,
  ): Promise<{ workflow: Workflow }> => {
    const raw = await request<{
      workflow: BackendWorkflow;
      stages?: BackendWorkflowStage[];
    }>("/workflows", { method: "POST", body: JSON.stringify(body) });
    return { workflow: workflowFromBackend(raw.workflow, raw.stages ?? []) };
  },
  setWorkflowActive: async (
    id: string,
    active: boolean,
  ): Promise<{ workflow: Workflow }> => {
    const raw = await request<{ workflow: BackendWorkflow }>(
      `/workflows/${encodeURIComponent(id)}`,
      { method: "PATCH", body: JSON.stringify({ active }) },
    );
    return { workflow: workflowFromBackend(raw.workflow) };
  },
  deleteWorkflow: (id: string) =>
    request<{ ok: boolean }>(`/workflows/${encodeURIComponent(id)}`, {
      method: "DELETE",
    }),
  getWorkflowRuns: (id: string, limit = 20) =>
    request<{ runs: WorkflowRun[] }>(
      `/workflows/${encodeURIComponent(id)}/runs?limit=${limit}`,
    ),
  // Workflow-scoped artifacts + verdicts (#302). One round-trip
  // replaces the dashboard's per-run artifact fan-out fetch and the
  // workspace-wide governance-events filter. Backend clamps `limit`
  // to [1, 500]; default 50 matches `/workflows/:id/runs`.
  getWorkflowArtifacts: (id: string, limit = 50) =>
    request<{ artifacts: SpineArtifact[] }>(
      `/workflows/${encodeURIComponent(id)}/artifacts?limit=${limit}`,
    ),
  getWorkflowVerdicts: (id: string, limit = 50) =>
    request<{ verdicts: GovernanceEvent[] }>(
      `/workflows/${encodeURIComponent(id)}/verdicts?limit=${limit}`,
    ),
  // Run detail hub (#303). Backend returns the projected run alongside
  // the parent workflow's backend shape; reuse `workflowFromBackend` so
  // the rest of the dashboard sees the same nested `Workflow` shape it
  // already consumes from `getWorkflow`.
  getRun: async (runId: string): Promise<RunDetail> => {
    const raw = await request<{
      run: WorkflowRun;
      workflow: BackendWorkflow;
      stages: BackendWorkflowStage[];
      sessions: RunLinkedSession[];
    }>(`/runs/${encodeURIComponent(runId)}`);
    const workflow = workflowFromBackend(raw.workflow, raw.stages);
    return {
      run: raw.run,
      workflow,
      stages: workflow.stages,
      sessions: raw.sessions,
    };
  },
  // Manual / replay trigger fires (#241 — Category 4 of the trigger
  // taxonomy umbrella #236). Both endpoints emit a `trigger.fired`
  // spine event the workflow runtime consumes, plus a
  // `workflow.manual_triggered` audit record.
  fireManualTrigger: (
    workflowId: string,
    name: string,
    payload?: Record<string, unknown>,
  ) =>
    request<{
      workflow_id: string;
      trigger_kind: "manual";
      name: string;
      trigger_event_id: number;
      actor: string;
    }>(
      `/workflows/${encodeURIComponent(workflowId)}/triggers/manual/${encodeURIComponent(name)}`,
      {
        method: "POST",
        body: JSON.stringify(payload === undefined ? {} : { payload }),
      },
    ),
  replayTrigger: (workflowId: string, sourceEventId: number) =>
    request<{
      workflow_id: string;
      trigger_kind: "replay";
      source_event_id: number;
      trigger_event_id: number;
      actor: string;
    }>(
      `/workflows/${encodeURIComponent(workflowId)}/triggers/replay/${sourceEventId}`,
      { method: "POST" },
    ),
  // Registry-backed workflow artifact kinds (issue #102). Poll-on-load; the
  // dashboard caches the result for the session. Falls back to the static
  // list in `workflow-meta.ts` if the fetch fails (offline / dev without
  // stiglab).
  listWorkflowKinds: () =>
    request<{ kinds: import("./types").WorkflowKindInfo[] }>("/workflow/kinds"),
};
