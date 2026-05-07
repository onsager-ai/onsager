import { request, scoped } from './client';
import type {
  GovernanceEvent,
  GovernanceStats,
  GovernanceRule,
  IsingInsightEmittedEvent,
  RuleProposal,
  SpineEvent,
} from './types';

export const governance = {
  // Governance (proxied to synodic). Workspace-scoped per #166; #164
  // tightens the backend to filter, today it's a forward-compat param.
  getGovernanceEvents: (workspaceId: string, type?: string) =>
    request<GovernanceEvent[]>(
      `/governance/events${scoped(workspaceId, { type })}`,
    ),
  getGovernanceStats: (workspaceId: string) =>
    request<GovernanceStats>(`/governance/stats${scoped(workspaceId)}`),
  getGovernanceRules: (workspaceId: string) =>
    request<GovernanceRule[]>(`/governance/rules${scoped(workspaceId)}`),
  // Ising insights — backed by the spine events endpoint (issue #36).
  // Returns a typed view of the `ising.insight_emitted` events so the
  // governance UI doesn't have to reach into each event's `data` blob.
  getIsingInsights: async (
    workspaceId: string,
    limit = 20,
  ): Promise<IsingInsightEmittedEvent[]> => {
    const res = await request<{ events: SpineEvent[] }>(
      `/spine/events${scoped(workspaceId, { event_type: 'ising.insight_emitted', limit })}`,
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
  getRuleProposals: (workspaceId: string, status?: RuleProposal['status']) =>
    request<RuleProposal[]>(
      `/governance/rule-proposals${scoped(workspaceId, { status })}`,
    ),
  resolveRuleProposal: (id: string, status: 'approved' | 'rejected', notes?: string) =>
    request<void>(`/governance/rule-proposals/${id}/resolve`, {
      method: 'PATCH',
      body: JSON.stringify({ status, notes }),
    }),
};
