import { request } from './client';
import type {
  ArtifactActionRequest,
  ArtifactActionResponse,
} from './types';
import type { ArtifactDetail } from './generated/ArtifactDetail';

export const artifacts = {
  getArtifact: (id: string) => request<{ artifact: ArtifactDetail }>(`/spine/artifacts/${id}`),
  abortArtifact: (id: string, body: ArtifactActionRequest = {}) =>
    request<ArtifactActionResponse>(`/spine/artifacts/${id}/abort`, {
      method: 'POST',
      body: JSON.stringify(body),
    }),
};
