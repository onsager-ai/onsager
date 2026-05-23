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
      // Operator-grain filter (ADR 0019 / spec #465). When `true`,
      // restrict to the registry's operator-grain allowlist. Defaults
      // to `false` on this generic endpoint; the Activity tab uses the
      // path-scoped `getActivity()` alias which bakes `true` in.
      operator_grain?: boolean;
      limit?: number;
    },
  ) =>
    request<{ events: SpineEvent[] }>(
      `/spine/events${scoped(workspaceId, params)}`,
    ),
  // Path-scoped Activity feed (ADR 0019 / spec #465). Same backing
  // table as `getSpineEvents`, but the workspace is on the path and
  // `operator_grain=true` is forced by the portal handler — the
  // Activity tab never has to remember to pass it.
  getActivity: (
    workspaceId: string,
    opts?: {
      stream_type?: string;
      event_type?: string;
      stream_id?: string;
      run_id?: string;
      limit?: number;
    },
  ) => {
    const params = new URLSearchParams();
    if (opts) {
      for (const [k, v] of Object.entries(opts)) {
        if (v != null && v !== '') params.set(k, String(v));
      }
    }
    const raw = params.toString();
    // `qs` is the lint-recognized variable name for query suffixes
    // in the api-contract matcher; leading `?` lives here so the
    // template stays a simple `${qs}` interpolation (#467 pattern).
    const qs = raw ? `?${raw}` : '';
    return request<{ events: SpineEvent[] }>(
      `/workspaces/${encodeURIComponent(workspaceId)}/activity${qs}`,
    );
  },
  getArtifacts: (
    workspaceId: string,
    filters?: { kind?: string; project_id?: string },
  ) =>
    request<{ artifacts: SpineArtifact[] }>(
      `/spine/artifacts${scoped(workspaceId, filters)}`,
    ),
};
