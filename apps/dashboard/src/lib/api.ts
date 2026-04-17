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
  // Governance (proxied to synodic)
  getGovernanceEvents: (type?: string) =>
    request<GovernanceEvent[]>(`/governance/events${type ? `?type=${type}` : ''}`),
  getGovernanceStats: () => request<GovernanceStats>('/governance/stats'),
  getGovernanceRules: () => request<GovernanceRule[]>('/governance/rules'),
  resolveGovernanceEvent: (id: string, notes?: string) =>
    request<void>(`/governance/events/${id}/resolve`, {
      method: 'PATCH',
      body: JSON.stringify({ notes }),
    }),
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
};
