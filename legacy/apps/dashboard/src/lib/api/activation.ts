// Activation funnel API client (spec #404).
//
// One typed surface for the dashboard-emitted FTUE events. The
// orchestration logic — dedup, anonymous_id, OSS opt-in — lives in
// `@/lib/activation` and calls `recordActivation` for the network hop.

import { request } from './client';

export interface RecordActivationBody {
  event: 'ftue.inspected' | 'ftue.drafted' | 'ftue.bound';
  occurred_at: string;
  anonymous_id: string;
  surface: 'landing' | 'chat' | 'dialog' | 'spine';
  path: 'cloud' | 'oss';
  context: Record<string, unknown>;
}

export interface RecordActivationResponse {
  recorded: boolean;
}

export const activation = {
  recordActivation: (body: RecordActivationBody) =>
    request<RecordActivationResponse>('/activation', {
      method: 'POST',
      body: JSON.stringify(body),
    }),
};
