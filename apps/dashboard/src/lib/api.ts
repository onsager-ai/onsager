const API_BASE = '/api';

export class ApiError extends Error {
  status: number;
  constructor(message: string, status: number) {
    super(message);
    this.status = status;
  }
}

async function request<T>(path: string, options?: RequestInit): Promise<T> {
  const res = await fetch(`${API_BASE}${path}`, {
    ...options,
    headers: {
      'Content-Type': 'application/json',
      ...options?.headers,
    },
  });

  if (!res.ok) {
    const error = await res.json().catch(() => ({ error: res.statusText }));
    throw new ApiError(error.error || res.statusText, res.status);
  }

  return res.json();
}

export interface Node {
  id: string;
  name: string;
  hostname: string;
  status: 'online' | 'offline' | 'draining';
  max_sessions: number;
  active_sessions: number;
  last_heartbeat: string;
  registered_at: string;
}

export interface Session {
  id: string;
  task_id: string;
  node_id: string;
  state: 'pending' | 'dispatched' | 'running' | 'waiting_input' | 'done' | 'failed';
  prompt: string;
  output: string | null;
  working_dir: string | null;
  created_at: string;
  updated_at: string;
}

export interface TaskRequest {
  prompt: string;
  node_id?: string;
  working_dir?: string;
  allowed_tools?: string[];
  max_turns?: number;
  project_id?: string;
}

export interface User {
  id: string;
  github_login: string;
  github_name: string | null;
  github_avatar_url: string | null;
}

export interface Credential {
  name: string;
  created_at: string;
  updated_at: string;
}

// Workspace (tenant) / membership / GitHub App installation / project
// types, issue #59 (Phase 0).
export interface Workspace {
  id: string;
  slug: string;
  name: string;
  created_by: string;
  created_at: string;
}

export interface WorkspaceMember {
  tenant_id: string;
  user_id: string;
  joined_at: string;
  github_login: string | null;
  github_name: string | null;
  github_avatar_url: string | null;
}

export type GitHubAccountType = 'user' | 'organization';

export interface GitHubAppInstallation {
  id: string;
  tenant_id: string;
  install_id: number;
  account_login: string;
  account_type: GitHubAccountType;
  created_at: string;
}

export interface Project {
  id: string;
  tenant_id: string;
  github_app_installation_id: string;
  repo_owner: string;
  repo_name: string;
  default_branch: string;
  created_at: string;
}

export interface AccessibleRepo {
  owner: string;
  name: string;
  default_branch: string | null;
  private: boolean;
}

// Governance types (proxied to synodic via /api/governance/*)
export interface GovernanceEvent {
  id: string;
  event_type: string;
  title: string;
  severity: string;
  source: string;
  metadata: Record<string, unknown>;
  resolved: boolean;
  resolution_notes: string | null;
  created_at: string;
  resolved_at: string | null;
}

export interface GovernanceStats {
  total: number;
  unresolved: number;
  by_type: Record<string, number>;
  by_severity: Record<string, number>;
}

export interface GovernanceRule {
  name: string;
  description: string;
  pattern: string;
  event_type: string;
  severity: string;
  enabled: boolean;
}

// Ising insight surface (issue #36). The emitter writes events to the spine
// as `ising.insight_emitted`; the dashboard reads them back via the spine
// events endpoint and presents the structured fields directly.
export interface IsingInsightEmittedEvent {
  id: number;
  created_at: string;
  signal_kind: string;
  subject_ref: string;
  confidence: number;
  evidence: { event_id: number; event_type: string }[];
}

// Ising rule proposal queue (issue #36 Step 2). Proxied through Synodic —
// each row corresponds to one `ising.rule_proposed` event the listener
// ingested and is awaiting human (or supervisor-agent) resolution.
export interface RuleProposal {
  id: string;
  insight_id: string;
  signal_kind: string;
  subject_ref: string;
  proposed_action: Record<string, unknown>;
  class: 'safe_auto' | 'review_required';
  rationale: string;
  confidence: number;
  status: 'pending' | 'approved' | 'rejected';
  resolution_notes: string | null;
  created_at: string;
  resolved_at: string | null;
}

// Token usage carried by `stiglab.session_completed` events (issue #39).
export interface TokenUsage {
  input_tokens: number;
  output_tokens: number;
  cache_read_tokens?: number;
  cache_write_tokens?: number;
  model?: string;
}

// Denormalized session-completion spend row (issue #39), projected from
// `stiglab.session_completed` spine events.
export interface SessionSpend {
  id: number;
  created_at: string;
  session_id: string;
  artifact_id: string | null;
  duration_ms: number;
  token_usage: TokenUsage | null;
}

// Spine types (direct from stiglab)
export interface SpineEvent {
  id: number;
  stream_id: string;
  stream_type: string;
  event_type: string;
  data: Record<string, unknown>;
  actor: string;
  created_at: string;
}

export interface SpineArtifact {
  id: string;
  kind: string;
  name: string;
  owner: string;
  state: string;
  current_version: number;
  consumers?: string[];
  created_at: string;
  updated_at: string;
}

export interface ArtifactDetail extends SpineArtifact {
  created_by: string;
  versions: ArtifactVersion[];
  vertical_lineage: ArtifactLineageEntry[];
  related_events?: SpineEvent[];
}

export interface ArtifactVersion {
  version: number;
  content_ref_uri: string;
  content_ref_checksum: string | null;
  change_summary: string;
  created_by_session: string;
  parent_version: number | null;
  created_at: string;
}

export interface ArtifactLineageEntry {
  version: number;
  session_id: string;
  recorded_at: string;
}

// Workflows (issue #82). A workflow is a trigger + an ordered list of stage
// cards. Triggers fire on external events (currently GitHub issue labels);
// each stage runs a gate that moves artifacts along — agent sessions,
// external checks, governance verdicts, or manual approvals.
//
// The CRUD API is delivered by a parallel sibling sub-issue of #79; this
// client is the typed surface the dashboard UI talks to.
//
// Artifact kinds are registry-backed as of #102. `WorkflowArtifactKind` is
// a string so any kind registered server-side is representable on the wire;
// the static-fallback list in `workflow-meta.ts` is what the UI renders
// when the runtime fetch fails (offline / dev without stiglab).
export type WorkflowArtifactKind = string;

export interface WorkflowTrigger {
  kind: 'github-label';
  install_id: string;
  repo_owner: string;
  repo_name: string;
  label: string;
}

export type WorkflowGateKind =
  | 'agent-session'
  | 'external-check'
  | 'governance'
  | 'manual-approval';

// Shape returned by `GET /api/workflow/kinds` (issue #102). The registry
// owns the canonical list; the dashboard's hardcoded set in
// `workflow-meta.ts` is only a fallback for offline/dev.
export type WorkflowMergeRule = 'overwrite' | 'merge_by_key' | 'append' | 'deep_merge';

// `intrinsic_schema` arrives as a `serde_json::Value`, which is any JSON
// value — including `null`, arrays, and primitives. Modelling it as
// `JsonValue` keeps the wire shape honest so consumers can't assume
// "always an object".
export type JsonValue =
  | string
  | number
  | boolean
  | null
  | { [key: string]: JsonValue }
  | JsonValue[];

export interface WorkflowKindInfo {
  id: string;
  description: string;
  merge_rule: WorkflowMergeRule;
  external_kind?: string;
  aliases: string[];
  intrinsic_schema: JsonValue;
}

export interface WorkflowStage {
  id: string;
  name: string;
  gate_kind: WorkflowGateKind;
  artifact_kind: WorkflowArtifactKind;
  config: Record<string, unknown>;
}

export type WorkflowStatus = 'draft' | 'active' | 'paused' | 'archived';

export interface Workflow {
  id: string;
  tenant_id: string;
  name: string;
  preset?: string | null;
  status: WorkflowStatus;
  trigger: WorkflowTrigger;
  stages: WorkflowStage[];
  created_at: string;
  updated_at: string;
}

// Wire contract for workflow CRUD. Matches stiglab's `CreateWorkflowBody`
// / `validate_create_body` exactly — flat trigger fields, numeric GitHub
// install id, snake_case `active`. Construct with `draftToCreateRequest`
// from the UI draft + installations list so the numeric id is resolved
// from the workspace installation record id the draft carries.
export interface CreateWorkflowRequest {
  tenant_id: string;
  name: string;
  trigger_kind: 'github-issue-webhook';
  repo_owner: string;
  repo_name: string;
  trigger_label: string;
  install_id: number;
  preset_id?: string;
  stages?: CreateWorkflowStage[];
  active: boolean;
}

export interface CreateWorkflowStage {
  gate_kind: WorkflowGateKind;
  params: Record<string, unknown>;
}

// Backend read shapes. Stiglab returns workflows with flat trigger fields
// and stages as `{ gate_kind, params }` with opaque JSON params. The UI
// keeps a richer nested `Workflow` shape; the adapters below translate
// so the rest of the app doesn't have to know the wire format.
interface BackendWorkflow {
  id: string;
  tenant_id: string;
  name: string;
  trigger_kind: 'github-issue-webhook';
  repo_owner: string;
  repo_name: string;
  trigger_label: string;
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

function workflowFromBackend(
  w: BackendWorkflow,
  stages: BackendWorkflowStage[] = [],
): Workflow {
  return {
    id: w.id,
    tenant_id: w.tenant_id,
    name: w.name,
    preset: w.preset_id,
    status: w.active ? 'active' : 'draft',
    trigger: {
      kind: 'github-label',
      install_id: String(w.install_id),
      repo_owner: w.repo_owner,
      repo_name: w.repo_name,
      label: w.trigger_label,
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

export type StageRunStatus = 'pending' | 'blocked' | 'passed' | 'failed';

export interface WorkflowRunStage {
  stage_id: string;
  status: StageRunStatus;
  updated_at: string;
}

export interface WorkflowRun {
  id: string;
  workflow_id: string;
  artifact_id: string | null;
  status: StageRunStatus;
  stages: WorkflowRunStage[];
  started_at: string;
  updated_at: string;
}

export interface GitHubLabel {
  name: string;
  color: string | null;
  description: string | null;
}

export interface RegisterArtifactRequest {
  kind: string;
  name: string;
  owner: string;
  description?: string;
  working_dir?: string;
}

export interface ArtifactActionRequest {
  reason?: string;
  actor?: string;
}

export interface OverrideGateRequestBody extends ArtifactActionRequest {
  verdict?: 'allow' | 'deny';
}

export interface ArtifactActionResponse {
  artifact_id: string;
  action: string;
  verdict?: string;
  reason?: string;
  escalation_id?: string;
}

export const api = {
  getNodes: () => request<{ nodes: Node[] }>('/nodes'),
  getSessions: () => request<{ sessions: Session[] }>('/sessions'),
  getSession: (id: string) => request<{ session: Session }>(`/sessions/${id}`),
  createTask: (task: TaskRequest) =>
    request<{ task: unknown; session: Session }>('/tasks', {
      method: 'POST',
      body: JSON.stringify(task),
    }),
  getHealth: () => request<{ status: string; version: string }>('/health'),
  // Auth
  getMe: () => request<{ user: User; auth_enabled: boolean }>('/auth/me'),
  logout: () =>
    request<{ ok: boolean }>('/auth/logout', { method: 'POST' }),
  // Credentials
  getCredentials: () =>
    request<{ credentials: Credential[] }>('/credentials'),
  setCredential: (name: string, value: string) =>
    request<{ ok: boolean }>(`/credentials/${encodeURIComponent(name)}`, {
      method: 'PUT',
      body: JSON.stringify({ value }),
    }),
  deleteCredential: (name: string) =>
    request<{ ok: boolean }>(`/credentials/${encodeURIComponent(name)}`, {
      method: 'DELETE',
    }),
  // Workspaces / tenants (issue #59)
  listWorkspaces: () => request<{ tenants: Workspace[] }>('/tenants'),
  createWorkspace: (body: { slug: string; name: string }) =>
    request<{ tenant: Workspace }>('/tenants', {
      method: 'POST',
      body: JSON.stringify(body),
    }),
  getWorkspace: (id: string) =>
    request<{ tenant: Workspace }>(`/tenants/${encodeURIComponent(id)}`),
  listWorkspaceMembers: (id: string) =>
    request<{ members: WorkspaceMember[] }>(
      `/tenants/${encodeURIComponent(id)}/members`,
    ),
  listWorkspaceInstallations: (id: string) =>
    request<{ installations: GitHubAppInstallation[] }>(
      `/tenants/${encodeURIComponent(id)}/github-installations`,
    ),
  registerWorkspaceInstallation: (
    id: string,
    body: {
      install_id: number;
      account_login: string;
      account_type: GitHubAccountType;
      webhook_secret?: string;
    },
  ) =>
    request<{ installation: GitHubAppInstallation }>(
      `/tenants/${encodeURIComponent(id)}/github-installations`,
      { method: 'POST', body: JSON.stringify(body) },
    ),
  deleteWorkspaceInstallation: (tenantId: string, installId: string) =>
    request<{ ok: boolean }>(
      `/tenants/${encodeURIComponent(tenantId)}/github-installations/${encodeURIComponent(installId)}`,
      { method: 'DELETE' },
    ),
  listWorkspaceProjects: (id: string) =>
    request<{ projects: Project[] }>(
      `/tenants/${encodeURIComponent(id)}/projects`,
    ),
  addWorkspaceProject: (
    id: string,
    body: {
      github_app_installation_id: string;
      repo_owner: string;
      repo_name: string;
      default_branch?: string;
    },
  ) =>
    request<{ project: Project }>(
      `/tenants/${encodeURIComponent(id)}/projects`,
      { method: 'POST', body: JSON.stringify(body) },
    ),
  listAllProjects: () => request<{ projects: Project[] }>('/projects'),
  deleteProject: (id: string) =>
    request<{ ok: boolean }>(`/projects/${encodeURIComponent(id)}`, {
      method: 'DELETE',
    }),
  // GitHub App install flow + accessible-repos picker (closes the last
  // Phase 0 items from #59: OAuth callback and the repo dropdown).
  getGitHubAppConfig: () =>
    request<{ enabled: boolean; slug?: string | null }>('/github-app/config'),
  listInstallationRepos: (tenantId: string, installId: string) =>
    request<{ repos: AccessibleRepo[] }>(
      `/tenants/${encodeURIComponent(tenantId)}/github-installations/${encodeURIComponent(installId)}/accessible-repos`,
    ),
  // Governance (proxied to synodic)
  getGovernanceEvents: (type?: string) =>
    request<GovernanceEvent[]>(`/governance/events${type ? `?type=${type}` : ''}`),
  getGovernanceStats: () => request<GovernanceStats>('/governance/stats'),
  getGovernanceRules: () => request<GovernanceRule[]>('/governance/rules'),
  // Ising insights — backed by the spine events endpoint (issue #36).
  // Returns a typed view of the `ising.insight_emitted` events so the
  // governance UI doesn't have to reach into each event's `data` blob.
  getIsingInsights: async (limit = 20): Promise<IsingInsightEmittedEvent[]> => {
    const res = await request<{ events: SpineEvent[] }>(
      `/spine/events?event_type=ising.insight_emitted&limit=${limit}`,
    );
    return res.events.map((e) => {
      const d = e.data as {
        signal_kind?: string;
        subject_ref?: string;
        confidence?: number;
        evidence?: { event_id: number; event_type: string }[];
      };
      return {
        id: e.id,
        created_at: e.created_at,
        signal_kind: d.signal_kind ?? 'unknown',
        subject_ref: d.subject_ref ?? '',
        confidence: typeof d.confidence === 'number' ? d.confidence : 0,
        evidence: Array.isArray(d.evidence) ? d.evidence : [],
      };
    });
  },
  resolveGovernanceEvent: (id: string, notes?: string) =>
    request<void>(`/governance/events/${id}/resolve`, {
      method: 'PATCH',
      body: JSON.stringify({ notes }),
    }),
  // Ising rule proposals (issue #36 Step 2). Served by Synodic and
  // proxied through stiglab's /api/governance/rule-proposals.
  getRuleProposals: (status?: RuleProposal['status']) =>
    request<RuleProposal[]>(
      `/governance/rule-proposals${status ? `?status=${status}` : ''}`,
    ),
  resolveRuleProposal: (id: string, status: 'approved' | 'rejected', notes?: string) =>
    request<void>(`/governance/rule-proposals/${id}/resolve`, {
      method: 'PATCH',
      body: JSON.stringify({ status, notes }),
    }),
  // Session spend view (issue #39). Reads recent `stiglab.session_completed`
  // events and unpacks the typed `token_usage` payload client-side so we
  // don't have to spin up a dedicated pricing/accounting endpoint just to
  // render the dashboard card.
  getSessionSpend: async (limit = 50): Promise<SessionSpend[]> => {
    const res = await request<{ events: SpineEvent[] }>(
      `/spine/events?event_type=stiglab.session_completed&limit=${limit}`,
    );
    return res.events.map((e) => {
      const d = e.data as {
        session_id?: string;
        artifact_id?: string | null;
        duration_ms?: number;
        token_usage?: TokenUsage;
      };
      return {
        id: e.id,
        created_at: e.created_at,
        session_id: d.session_id ?? '',
        artifact_id: d.artifact_id ?? null,
        duration_ms: typeof d.duration_ms === 'number' ? d.duration_ms : 0,
        token_usage: d.token_usage ?? null,
      };
    });
  },
  // Spine
  getSpineEvents: (params?: { stream_type?: string; event_type?: string; limit?: number }) => {
    const qs = params ? '?' + new URLSearchParams(
      Object.entries(params).filter(([, v]) => v != null).map(([k, v]) => [k, String(v)])
    ).toString() : '';
    return request<{ events: SpineEvent[] }>(`/spine/events${qs}`);
  },
  getArtifacts: () => request<{ artifacts: SpineArtifact[] }>('/spine/artifacts'),
  getArtifact: (id: string) => request<{ artifact: ArtifactDetail }>(`/spine/artifacts/${id}`),
  registerArtifact: (req: RegisterArtifactRequest) =>
    request<{ artifact: SpineArtifact }>('/spine/artifacts', {
      method: 'POST',
      body: JSON.stringify(req),
    }),
  retryArtifact: (id: string, body: ArtifactActionRequest = {}) =>
    request<ArtifactActionResponse>(`/spine/artifacts/${id}/retry`, {
      method: 'POST',
      body: JSON.stringify(body),
    }),
  abortArtifact: (id: string, body: ArtifactActionRequest = {}) =>
    request<ArtifactActionResponse>(`/spine/artifacts/${id}/abort`, {
      method: 'POST',
      body: JSON.stringify(body),
    }),
  overrideGate: (id: string, body: OverrideGateRequestBody = {}) =>
    request<ArtifactActionResponse>(`/spine/artifacts/${id}/override-gate`, {
      method: 'POST',
      body: JSON.stringify(body),
    }),
  // Workflows (issue #82) — CRUD + live runs. The API is provided by the
  // stiglab sibling sub-issue; the dashboard is the only client today.
  // The backend persists workflows with flat trigger fields and stage `params`;
  // these wrappers translate backend → UI shape.
  listWorkflows: async (tenantId: string): Promise<{ workflows: Workflow[] }> => {
    if (!tenantId) throw new ApiError('tenantId is required', 400);
    const raw = await request<{ workflows: BackendWorkflow[] }>(
      `/workflows?tenant_id=${encodeURIComponent(tenantId)}`,
    );
    return { workflows: raw.workflows.map((w) => workflowFromBackend(w)) };
  },
  // Fan-out across every workspace the user belongs to. Stiglab's list
  // endpoint is tenant-scoped; cross-tenant "do I have any workflows yet?"
  // queries (empty-state gates, first-run redirect) need this shape. We
  // hit `/tenants` once and one `/workflows?tenant_id=…` per workspace;
  // fine for the workspace counts we target (typically 1–3).
  listWorkflowsForUser: async (): Promise<{ workflows: Workflow[] }> => {
    const { tenants } = await request<{ tenants: { id: string }[] }>('/tenants');
    const lists = await Promise.all(
      tenants.map((t) =>
        request<{ workflows: BackendWorkflow[] }>(
          `/workflows?tenant_id=${encodeURIComponent(t.id)}`,
        ).then((r) => r.workflows.map((w) => workflowFromBackend(w))),
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
  // Registry-backed workflow artifact kinds (issue #102). Poll-on-load; the
  // dashboard caches the result for the session. Falls back to the static
  // list in `workflow-meta.ts` if the fetch fails (offline / dev without
  // stiglab).
  listWorkflowKinds: () =>
    request<{ kinds: WorkflowKindInfo[] }>('/workflow/kinds'),
  // GitHub labels for a workspace install + repo. Used by the trigger card
  // combobox so the user selects from existing labels (with an inline
  // create-new affordance) instead of free-texting.
  listRepoLabels: (tenantId: string, installId: string, owner: string, repo: string) =>
    request<{ labels: GitHubLabel[] }>(
      `/tenants/${encodeURIComponent(tenantId)}/github-installations/${encodeURIComponent(installId)}/repos/${encodeURIComponent(owner)}/${encodeURIComponent(repo)}/labels`,
    ),
};
