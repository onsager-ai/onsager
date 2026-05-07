import { request } from './client';
import type { EventManifestEntry, TriggerManifestEntry } from './types';

export const registry = {
  // Event-type registry manifest (spec #131 Lever E / #150). Returns
  // every `FactoryEventKind` variant with its declared producers and
  // consumers — the dashboard renders this in the architecture / event
  // catalog view so reviewers can see the cross-subsystem contract
  // without reading source.
  listEventManifest: () =>
    request<{ events: EventManifestEntry[] }>('/registry/events'),
  // Trigger-kind registry manifest (spec #237). Returns every registered
  // `TriggerKind` variant with its category, UI shape, and description.
  // Drives `<TriggerKindPicker>` so the dashboard picks kinds from the
  // catalog instead of hardcoding the union.
  listTriggerManifest: () =>
    request<{ triggers: TriggerManifestEntry[] }>('/registry/triggers'),
};
