import { ApiError, request } from './client';
import type {
  GovernanceEvent,
  SpineArtifact,
  Workflow,
  WorkflowStage,
  WorkflowGateKind,
  WorkflowArtifactKind,
  CreateWorkflowRequest,
  CreateWorkflowStage,
  WorkflowRun,
  RunDetail,
  RunLinkedSession,
} from './types';

// Backend read shapes. Stiglab returns workflows with the unified
// `trigger` variant (spec #237) — `{ kind: 'github_issue_webhook',
// repo, label }` — and stages as `{ gate_kind, params }` with opaque
// JSON params. The UI keeps a richer nested `Workflow` shape; the
// adapters below translate so the rest of the app doesn't have to
// know the wire format.
interface BackendTrigger {
  kind: string;
  repo?: string;
  label?: string;
  // Manual / replay variants — `name` for `manual`, `source_event_id`
  // for `replay`. Other variants carry their own per-kind fields the
  // UI doesn't currently render.
  name?: string;
  [extra: string]: unknown;
}

interface BackendWorkflow {
  id: string;
  workspace_id: string;
  name: string;
  trigger: BackendTrigger;
  install_id: number;
  preset_id: string | null;
  active: boolean;
  created_at: string;
  updated_at: string;
}

interface BackendWorkflowStage {
  id: string;
  workflow_id: string;
  seq: number;
  gate_kind: WorkflowGateKind;
  params: Record<string, unknown>;
}

// Legacy `Spec` / `github-issue` / `PullRequest` / `github-pr` get folded
// into the canonical `Issue` / `PR` names (issue #102). Anything else
// passes through unchanged — registered custom kinds keep their id.
export function normalizeWorkflowArtifactKind(kind: string): WorkflowArtifactKind {
  switch (kind) {
    case 'github-issue':
    case 'Spec':
      return 'Issue';
    case 'github-pr':
    case 'PullRequest':
      return 'PR';
    default:
      return kind;
  }
}

// Pack a UI stage into the backend's `{ gate_kind, params }` pair. UI-only
// display fields ride in `params` so they survive the round-trip without
// a backend-schema change.
export function stageToCreateStage(s: WorkflowStage): CreateWorkflowStage {
  return {
    gate_kind: s.gate_kind,
    params: {
      ...(s.config ?? {}),
      name: s.name,
      artifact_kind: s.artifact_kind,
    },
  };
}

function stageFromBackend(s: BackendWorkflowStage): WorkflowStage {
  const params = (s.params ?? {}) as Record<string, unknown>;
  const name = typeof params.name === 'string' ? params.name : undefined;
  // Registry-backed kinds (#102) — accept any string; normalize legacy values.
  const rawKind = typeof params.artifact_kind === 'string' ? params.artifact_kind : 'Issue';
  const artifactKind = normalizeWorkflowArtifactKind(rawKind);
  // Everything except the UI-only display fields is opaque stage config.
  const { name: _n, artifact_kind: _a, ...config } = params as Record<string, unknown>;
  void _n;
  void _a;
  return {
    id: s.id,
    name: name ?? defaultStageName(s.gate_kind),
    gate_kind: s.gate_kind,
    artifact_kind: artifactKind,
    config,
  };
}

function workflowFromBackend(
  w: BackendWorkflow,
  stages: BackendWorkflowStage[] = [],
): Workflow {
  // Today the only kind is `github_issue_webhook`; the registry
  // (`/api/registry/triggers`) is the source of truth for which kinds
  // exist. Per-kind UI translation lives here so the rest of the app
  // can keep its richer nested trigger shape.
  const repo = w.trigger.repo ?? '';
  const [repoOwner = '', repoName = ''] = repo.split('/');
  return {
    id: w.id,
    workspace_id: w.workspace_id,
    name: w.name,
    preset: w.preset_id,
    status: w.active ? 'active' : 'draft',
    trigger: {
      kind: 'github-label',
      install_id: String(w.install_id),
      repo_owner: repoOwner,
      repo_name: repoName,
      label: w.trigger.label ?? '',
      kind_tag: w.trigger.kind ?? '',
      manual_name: typeof w.trigger.name === 'string' ? w.trigger.name : '',
    },
    stages: stages.map(stageFromBackend),
    created_at: w.created_at,
    updated_at: w.updated_at,
  };
}

function defaultStageName(gate: WorkflowGateKind): string {
  switch (gate) {
    case 'agent-session':
      return 'Agent session';
    case 'external-check':
      return 'CI check';
    case 'governance':
      return 'Governance';
    case 'manual-approval':
      return 'Manual approval';
  }
}

export const workflows = {
  // Workflows (issue #82) — CRUD + live runs. The API is provided by the
  // stiglab sibling sub-issue; the dashboard is the only client today.
  // The backend persists workflows with flat trigger fields and stage `params`;
  // these wrappers translate backend → UI shape.
  listWorkflows: async (workspaceId: string): Promise<{ workflows: Workflow[] }> => {
    if (!workspaceId) throw new ApiError('workspaceId is required', 400);
    const raw = await request<{ workflows: BackendWorkflow[] }>(
      `/workflows?workspace_id=${encodeURIComponent(workspaceId)}`,
    );
    return { workflows: raw.workflows.map((w) => workflowFromBackend(w)) };
  },
  // Fan-out across every workspace the user belongs to. Stiglab's list
  // endpoint is workspace-scoped; cross-workspace "do I have any workflows
  // yet?" queries (empty-state gates, first-run redirect) need this shape.
  // We hit `/workspaces` once and one `/workflows?workspace_id=…` per
  // workspace; fine for the workspace counts we target (typically 1–3).
  listWorkflowsForUser: async (): Promise<{ workflows: Workflow[] }> => {
    const { workspaces } = await request<{ workspaces: { id: string }[] }>(
      '/workspaces',
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
  getWorkflow: async (id: string): Promise<{ workflow: Workflow }> => {
    const raw = await request<{ workflow: BackendWorkflow; stages: BackendWorkflowStage[] }>(
      `/workflows/${encodeURIComponent(id)}`,
    );
    return { workflow: workflowFromBackend(raw.workflow, raw.stages) };
  },
  createWorkflow: async (body: CreateWorkflowRequest): Promise<{ workflow: Workflow }> => {
    const raw = await request<{ workflow: BackendWorkflow; stages?: BackendWorkflowStage[] }>(
      '/workflows',
      { method: 'POST', body: JSON.stringify(body) },
    );
    return { workflow: workflowFromBackend(raw.workflow, raw.stages ?? []) };
  },
  setWorkflowActive: async (id: string, active: boolean): Promise<{ workflow: Workflow }> => {
    const raw = await request<{ workflow: BackendWorkflow }>(
      `/workflows/${encodeURIComponent(id)}`,
      { method: 'PATCH', body: JSON.stringify({ active }) },
    );
    return { workflow: workflowFromBackend(raw.workflow) };
  },
  deleteWorkflow: (id: string) =>
    request<{ ok: boolean }>(`/workflows/${encodeURIComponent(id)}`, {
      method: 'DELETE',
    }),
  getWorkflowRuns: (id: string, limit = 20) =>
    request<{ runs: WorkflowRun[] }>(
      `/workflows/${encodeURIComponent(id)}/runs?limit=${limit}`,
    ),
  // Workflow-scoped artifacts + verdicts (#302). One round-trip
  // replaces the dashboard's per-run artifact fan-out fetch and the
  // workspace-wide governance-events filter.
  getWorkflowArtifacts: (id: string) =>
    request<{ artifacts: SpineArtifact[] }>(
      `/workflows/${encodeURIComponent(id)}/artifacts`,
    ),
  getWorkflowVerdicts: (id: string) =>
    request<{ verdicts: GovernanceEvent[] }>(
      `/workflows/${encodeURIComponent(id)}/verdicts`,
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
      trigger_kind: 'manual';
      name: string;
      trigger_event_id: number;
      actor: string;
    }>(
      `/workflows/${encodeURIComponent(workflowId)}/triggers/manual/${encodeURIComponent(name)}`,
      {
        method: 'POST',
        body: JSON.stringify(payload === undefined ? {} : { payload }),
      },
    ),
  replayTrigger: (workflowId: string, sourceEventId: number) =>
    request<{
      workflow_id: string;
      trigger_kind: 'replay';
      source_event_id: number;
      trigger_event_id: number;
      actor: string;
    }>(
      `/workflows/${encodeURIComponent(workflowId)}/triggers/replay/${sourceEventId}`,
      { method: 'POST' },
    ),
  // Registry-backed workflow artifact kinds (issue #102). Poll-on-load; the
  // dashboard caches the result for the session. Falls back to the static
  // list in `workflow-meta.ts` if the fetch fails (offline / dev without
  // stiglab).
  listWorkflowKinds: () =>
    request<{ kinds: import('./types').WorkflowKindInfo[] }>('/workflow/kinds'),
};
