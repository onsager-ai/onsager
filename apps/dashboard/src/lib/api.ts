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
