import { request, scoped } from './client';
import type { Node, Session, TaskRequest, SessionKind, User, SessionSpend, SpineEvent, TokenUsage } from './types';

export const sessions = {
  getNodes: (workspaceId: string) =>
    request<{ nodes: Node[] }>(`/nodes${scoped(workspaceId)}`),
  getSession: (id: string) => request<{ session: Session }>(`/sessions/${id}`),
  /**
   * Request cancellation of an in-flight session (#303). Best-effort —
   * portal emits `portal.session_cancel_requested` and returns 202; the
   * UI should not block on the agent actually stopping.
   */
  cancelSession: (id: string) =>
    request<{ ok: boolean }>(`/sessions/${encodeURIComponent(id)}/cancel`, {
      method: 'POST',
    }),
  createTask: (task: TaskRequest) =>
    request<{ task: unknown; session: Session }>('/tasks', {
      method: 'POST',
      body: JSON.stringify(task),
    }),
  getHealth: () => request<{ status: string; version: string }>('/health'),
  // Auth
  getMe: () =>
    request<{ user: User; session_kind: SessionKind; via?: 'session' | 'pat' }>(
      '/auth/me',
    ),
  logout: () =>
    request<{ ok: boolean }>('/auth/logout', { method: 'POST' }),
  /**
   * Mint a session for the seeded `${USER}@local` dev user (issue #193).
   * 404s in release builds — the route is `cfg(debug_assertions)`-gated
   * server-side. The LoginPage probes for that 404 to decide whether to
   * render the "Dev Login" button.
   */
  devLogin: () =>
    request<{ ok: boolean; session_kind: SessionKind; user: User }>(
      '/auth/dev-login',
      { method: 'POST' },
    ),
  authProviders: () =>
    request<{ github: boolean; dev: boolean }>('/auth/providers'),
  // Session spend view (issue #39). Reads recent `stiglab.session_completed`
  // events and unpacks the typed `token_usage` payload client-side so we
  // don't have to spin up a dedicated pricing/accounting endpoint just to
  // render the dashboard card.
  getSessionSpend: async (
    workspaceId: string,
    limit = 50,
  ): Promise<SessionSpend[]> => {
    const res = await request<{ events: SpineEvent[] }>(
      `/spine/events${scoped(workspaceId, { event_type: 'stiglab.session_completed', limit })}`,
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
};
