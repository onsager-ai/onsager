import { request } from './client';
import type {
  ArtifactActionRequest,
  ArtifactActionResponse,
  OverrideGateRequestBody,
} from './types';
import type { ArtifactDetail } from './generated/ArtifactDetail';

export const artifacts = {
  getArtifact: (id: string) => request<{ artifact: ArtifactDetail }>(`/spine/artifacts/${id}`),
  retryArtifact: (id: string, body: ArtifactActionRequest = {}) =>
    request<ArtifactActionResponse>(`/spine/artifacts/${id}/retry`, {
      method: 'POST',
      body: JSON.stringify(body),
    }),
  abortArtifact: (id: string, body: ArtifactActionRequest = {}) =>
    request<ArtifactActionResponse>(`/spine/artifacts/${id}/abort`, {
      method: 'POST',
      body: JSON.stringify(body),
    }),
  overrideGate: (id: string, body: OverrideGateRequestBody = {}) =>
    request<ArtifactActionResponse>(`/spine/artifacts/${id}/override-gate`, {
      method: 'POST',
      body: JSON.stringify(body),
    }),
};
