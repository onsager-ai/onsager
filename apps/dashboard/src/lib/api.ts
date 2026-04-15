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
};
