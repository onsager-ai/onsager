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
      // Filter by run id (spec #303). A run is one artifact flowing
      // through a workflow; backend joins this against `stream_id`,
      // `data->>'artifact_id'`, and the `sessions.artifact_id` lookup
      // so session-keyed events still surface.
      run_id?: string;
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
