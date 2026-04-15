const BASE = '/api';

export interface Event {
  id: string;
  event_type: string;
  title: string;
  severity: string;
  source: string;
  metadata: Record<string, unknown>;
  resolved: boolean;
  resolution_notes: string | null;
  created_at: string;
  resolved_at: string | null;
}

export interface Stats {
  total: number;
  unresolved: number;
  by_type: Record<string, number>;
  by_severity: Record<string, number>;
}

export interface Rule {
  name: string;
  description: string;
  pattern: string;
  event_type: string;
  severity: string;
  enabled: boolean;
}

export async function fetchEvents(params?: Record<string, string>): Promise<Event[]> {
  const qs = params ? '?' + new URLSearchParams(params).toString() : '';
  const res = await fetch(`${BASE}/events${qs}`);
  return res.json();
}

export async function fetchEvent(id: string): Promise<Event> {
  const res = await fetch(`${BASE}/events/${id}`);
  return res.json();
}

export async function submitEvent(body: { type: string; title: string; severity?: string; source?: string }): Promise<Event> {
  const res = await fetch(`${BASE}/events`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(body),
  });
  return res.json();
}

export async function resolveEvent(id: string, notes?: string): Promise<void> {
  await fetch(`${BASE}/events/${id}/resolve`, {
    method: 'PATCH',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ notes }),
  });
}

export async function fetchStats(): Promise<Stats> {
  const res = await fetch(`${BASE}/stats`);
  return res.json();
}

export async function fetchRules(): Promise<Rule[]> {
  const res = await fetch(`${BASE}/rules`);
  return res.json();
}
