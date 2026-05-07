import { request } from './client';
import type { Credential } from './types';

export const credentials = {
  // Credentials live under /api/workspaces/:workspace/credentials (#189).
  getCredentials: (workspaceId: string) =>
    request<{ credentials: Credential[] }>(
      `/workspaces/${encodeURIComponent(workspaceId)}/credentials`,
    ),
  setCredential: (workspaceId: string, name: string, value: string) =>
    request<{ ok: boolean }>(
      `/workspaces/${encodeURIComponent(workspaceId)}/credentials/${encodeURIComponent(name)}`,
      {
        method: 'PUT',
        body: JSON.stringify({ value }),
      },
    ),
  deleteCredential: (workspaceId: string, name: string) =>
    request<{ ok: boolean }>(
      `/workspaces/${encodeURIComponent(workspaceId)}/credentials/${encodeURIComponent(name)}`,
      { method: 'DELETE' },
    ),
};
