import { request } from './client';
import type { ProjectPullRow, ProjectLiveListResponse } from './types';

export const pulls = {
  listProjectPulls: (projectId: string, state?: 'open' | 'closed' | 'all') => {
    const qs = state ? `?state=${state}` : '';
    return request<ProjectLiveListResponse<ProjectPullRow>>(
      `/projects/${encodeURIComponent(projectId)}/pulls${qs}`,
    );
  },
};
