import { request } from './client';
import type { Pat, CreatePatResponse } from './types';

export const pats = {
  // Personal Access Tokens (issue #143)
  listPats: () => request<{ pats: Pat[] }>('/pats'),
  createPat: (body: {
    name: string;
    workspace_id: string;
    // v1: an explicit ISO-8601 future timestamp is required. The "never
    // expires" affordance is intentionally not exposed in this release.
    expires_at: string;
  }) =>
    request<CreatePatResponse>('/pats', {
      method: 'POST',
      body: JSON.stringify(body),
    }),
  revokePat: (id: string) =>
    request<{ ok: boolean }>(`/pats/${encodeURIComponent(id)}`, {
      method: 'DELETE',
    }),
};
