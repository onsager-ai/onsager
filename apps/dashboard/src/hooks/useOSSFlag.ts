// Cloud-vs-OSS capability boundary (spec #405).
//
// Returns `true` when the portal reports `is_oss: true` via
// `GET /api/build-info`. Wraps `useBuildInfo` so callers can ask "are
// we OSS?" without ceremony: undefined / loading / errored all resolve
// to `false`, matching the anti-nagware default (when in doubt, the
// surface is silent, not promotional).
//
// Used at the three Cloud-vs-OSS surfacing points:
//   - Workflow detail page Runs tab — 7-day retention cap line.
//   - Workflow detail page trigger panel — scheduler limitation line.
//   - Chat draft strip — "Drafts on this device." footer.

import { useBuildInfo } from "@/lib/build-info"

export function useOSSFlag(): boolean {
  const buildInfo = useBuildInfo()
  return buildInfo?.is_oss ?? false
}
