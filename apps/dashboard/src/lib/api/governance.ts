import { request, scoped } from './client';
import type {
  GovernanceEvent,
} from './types';

export const governance = {
  // Governance (proxied to synodic). Workspace-scoped per #166; #164
  // tightens the backend to filter, today it's a forward-compat param.
  getGovernanceEvents: (workspaceId: string, type?: string) =>
    request<GovernanceEvent[]>(
      `/governance/events${scoped(workspaceId, { type })}`,
    ),
  resolveGovernanceEvent: (id: string, notes?: string) =>
    request<void>(`/governance/events/${id}/resolve`, {
      method: 'PATCH',
      body: JSON.stringify({ notes }),
    }),
};
