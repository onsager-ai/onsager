import { request } from './client';
import type {
  ProjectIssueDetailResponse,
  BackfillRequestBody,
  BackfillReport,
  ReplayIssueTriggerRequest,
  ReplayIssueTriggerResponse,
} from './types';

export const issues = {
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
