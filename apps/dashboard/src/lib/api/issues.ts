import { request } from './client';
import type {
  ProjectIssueRow,
  ProjectIssueDetailResponse,
  ProjectLiveListResponse,
  BackfillRequestBody,
  BackfillReport,
  ReplayIssueTriggerRequest,
  ReplayIssueTriggerResponse,
} from './types';

export const issues = {
  // Live-hydration proxy endpoints (specs #167, #170, #171). Dashboard
  // joins skeleton rows from `getArtifacts({kind: ...})` with the rows
  // returned here on `external_ref`.
  listProjectIssues: (projectId: string, state?: 'open' | 'closed' | 'all') => {
    const qs = state ? `?state=${state}` : '';
    return request<ProjectLiveListResponse<ProjectIssueRow>>(
      `/projects/${encodeURIComponent(projectId)}/issues${qs}`,
    );
  },
  getProjectIssue: (projectId: string, number: number) =>
    request<ProjectIssueDetailResponse>(
      `/projects/${encodeURIComponent(projectId)}/issues/${number}`,
    ),
  backfillProject: (projectId: string, body: BackfillRequestBody = {}) =>
    request<BackfillReport>(`/projects/${encodeURIComponent(projectId)}/backfill`, {
      method: 'POST',
      body: JSON.stringify(body),
    }),
  replayIssueTrigger: (
    projectId: string,
    issueNumber: number,
    body: ReplayIssueTriggerRequest = {},
  ) =>
    request<ReplayIssueTriggerResponse>(
      `/projects/${encodeURIComponent(projectId)}/issues/${issueNumber}/replay-trigger`,
      {
        method: 'POST',
        body: JSON.stringify(body),
      },
    ),
};
