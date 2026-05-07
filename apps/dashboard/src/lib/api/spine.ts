import { request, scoped } from './client';
import type { SpineEvent, SpineArtifact } from './types';

export const spine = {
  // Spine
  getSpineEvents: (
    workspaceId: string,
    params?: {
      stream_type?: string;
      event_type?: string;
      stream_id?: string;
      limit?: number;
    },
  ) =>
    request<{ events: SpineEvent[] }>(
      `/spine/events${scoped(workspaceId, params)}`,
    ),
  getArtifacts: (
    workspaceId: string,
    filters?: { kind?: string; project_id?: string },
  ) =>
    request<{ artifacts: SpineArtifact[] }>(
      `/spine/artifacts${scoped(workspaceId, filters)}`,
    ),
};
