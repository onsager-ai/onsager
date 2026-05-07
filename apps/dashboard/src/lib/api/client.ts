export const API_BASE = '/api';

export class ApiError extends Error {
  status: number;
  constructor(message: string, status: number) {
    super(message);
    this.status = status;
  }
}

export async function request<T>(path: string, options?: RequestInit): Promise<T> {
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

// Build a `?workspace_id=...&k=v` query string. Workspace scope is the
// universal first-class filter for scoped lists (#166); the backend
// adds enforcement once #164 lands. Today, endpoints that don't yet
// filter on `workspace_id` simply ignore the param — passing it now
// keeps the wire format ready and the React Query keys honest.
export function scoped(
  workspaceId: string,
  extra?: Record<string, string | number | undefined | null>,
): string {
  const params = new URLSearchParams({ workspace_id: workspaceId });
  if (extra) {
    for (const [k, v] of Object.entries(extra)) {
      if (v != null && v !== '') params.set(k, String(v));
    }
  }
  return `?${params.toString()}`;
}
