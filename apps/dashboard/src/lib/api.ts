// v2 API client — hand-rolled minimal surface for the M0 shell.
// Grows with each milestone; types stay co-located until the surface
// is big enough to justify regenerating them from Rust (legacy ts-rs
// SSOT pattern, re-judged at M1).

export const API_BASE = '/api'

export class ApiError extends Error {
  status: number
  constructor(message: string, status: number) {
    super(message)
    this.status = status
  }
}

export async function request<T>(path: string, options?: RequestInit): Promise<T> {
  const res = await fetch(`${API_BASE}${path}`, {
    ...options,
    headers: {
      'Content-Type': 'application/json',
      ...options?.headers,
    },
  })
  if (!res.ok) {
    const error = await res.json().catch(() => ({ error: res.statusText }))
    throw new ApiError(error.error || res.statusText, res.status)
  }
  return res.json()
}

export type SessionKind = 'github' | 'dev'

export interface MeUser {
  id: string
  github_login: string
  github_name: string | null
  github_avatar_url: string | null
}

export interface Workspace {
  id: string
  slug: string
  name: string
  created_by: string
  created_at: string
}

export interface Credential {
  name: string
  created_at: string
  updated_at: string
}

export const api = {
  authProviders: () => request<{ github: boolean; dev: boolean }>('/auth/providers'),
  getMe: () => request<{ session_kind: SessionKind; user: MeUser }>('/auth/me'),
  devLogin: () => request<{ ok: boolean }>('/auth/dev-login', { method: 'POST' }),
  logout: () => request<{ ok: boolean }>('/auth/logout', { method: 'POST' }),
  listWorkspaces: () => request<{ workspaces: Workspace[] }>('/workspaces'),
  listCredentials: (workspaceId: string) =>
    request<{ credentials: Credential[] }>(`/workspaces/${workspaceId}/credentials`),
  setCredential: (workspaceId: string, name: string, value: string) =>
    request<{ ok: boolean }>(`/workspaces/${workspaceId}/credentials/${name}`, {
      method: 'PUT',
      body: JSON.stringify({ value }),
    }),
  deleteCredential: (workspaceId: string, name: string) =>
    request<{ ok: boolean }>(`/workspaces/${workspaceId}/credentials/${name}`, {
      method: 'DELETE',
    }),
}

// ── M1: workflows / runs / artifacts ──

export interface StageAgent {
  kind: 'agent'
  model?: string | null
  system_prompt?: string | null
  user_prompt: string
}
export type Stage = StageAgent
export interface TriggerManual {
  kind: 'manual'
  name: string
}
export interface TriggerGithubIssue {
  kind: 'github_issue_webhook'
  repo: string
  label: string
}
export type Trigger = TriggerManual | TriggerGithubIssue

export interface Workflow {
  id: string
  workspace_id: string
  name: string
  trigger: Trigger
  definition: Stage[]
  active: boolean
  created_by: string
  created_at: string
  updated_at: string
}

export interface Run {
  id: string
  workspace_id: string
  workflow_id: string
  status: string
  error: string | null
  started_at: string
  finished_at: string | null
}

export interface RunEvent {
  id: number
  kind: string
  payload: Record<string, unknown>
  created_at: string
}

export interface Artifact {
  id: string
  workspace_id: string
  run_id: string | null
  session_id: string | null
  kind: string
  name: string
  content_uri: string | null
  content_checksum: string | null
  external_ref: Record<string, unknown> | null
  provenance: { kind: string; source: string }
  created_at: string
}

export const workflowsApi = {
  list: (workspaceId: string) =>
    request<{ workflows: Workflow[] }>(`/workflows?workspace=${workspaceId}`),
  create: (body: {
    workspace_id: string
    name: string
    trigger: Trigger
    definition: Stage[]
  }) => request<{ workflow: Workflow }>('/workflows', { method: 'POST', body: JSON.stringify(body) }),
  get: (id: string) => request<{ workflow: Workflow }>(`/workflows/${id}`),
  setActive: (id: string, active: boolean) =>
    request<{ workflow: Workflow }>(`/workflows/${id}`, {
      method: 'PATCH',
      body: JSON.stringify({ active }),
    }),
  fire: (id: string) => request<{ run_id: string }>(`/workflows/${id}/fire`, { method: 'POST' }),
  runs: (id: string) => request<{ runs: Run[] }>(`/workflows/${id}/runs`),
}

export const runsApi = {
  get: (id: string) =>
    request<{
      run: Run
      timeline: RunEvent[]
      artifacts: Artifact[]
      sessions: { id: string; status: string }[]
    }>(`/runs/${id}`),
}

export const artifactsApi = {
  list: (workspaceId: string) =>
    request<{ artifacts: Artifact[] }>(`/artifacts?workspace=${workspaceId}`),
  get: (id: string) => request<{ artifact: Artifact }>(`/artifacts/${id}`),
}

/**
 * Decode an `inline:base64,...` content URI to text; null otherwise.
 * `atob` alone yields latin-1 and mangles multi-byte UTF-8 (em-dashes
 * became `â`), so the bytes go through TextDecoder.
 */
export function decodeInlineContent(uri: string | null): string | null {
  if (!uri?.startsWith('inline:base64,')) return null
  try {
    const bytes = Uint8Array.from(atob(uri.slice('inline:base64,'.length)), (c) =>
      c.charCodeAt(0),
    )
    return new TextDecoder().decode(bytes)
  } catch {
    return null
  }
}

// ── M3: activity / abort / credentials ──

export interface ActivityEvent {
  id: number
  run_id: string | null
  kind: string
  payload: Record<string, unknown>
  created_at: string
}

export const operateApi = {
  activity: (workspaceId: string) =>
    request<{ events: ActivityEvent[] }>(`/activity?workspace=${workspaceId}`),
  abortRun: (runId: string) =>
    request<{ ok: boolean; status: string }>(`/runs/${runId}/abort`, { method: 'POST' }),
}

// ── Session feed (#632) ──

export interface SessionEvent {
  seq: number
  kind: string
  payload: Record<string, unknown>
  created_at: string
}

export interface RunSession {
  id: string
  status: string
}

export const sessionsApi = {
  events: (sessionId: string) =>
    request<{ status: string; events: SessionEvent[] }>(`/sessions/${sessionId}/events`),
}

// ── Fleet (ADR 0030 M4) ──

export interface Machine {
  name: string
  status: string
  connected_at: string | null
  last_seen_at: string
  in_flight: number
}

export const fleetApi = {
  listMachines: () => request<{ machines: Machine[] }>('/fleet/machines'),
}
