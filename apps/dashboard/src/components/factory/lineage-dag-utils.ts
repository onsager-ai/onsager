import type { ArtifactVersion, SpineEvent } from "@/lib/api"

// Group spine events into per-run lanes keyed by shaping request_id.
export function buildLanes(
  events: SpineEvent[] | undefined,
): Map<string, SpineEvent[]> {
  const lanes = new Map<string, SpineEvent[]>()
  if (!events) return lanes
  for (const e of events) {
    const requestId = e.data.request_id as string | undefined
    if (!requestId) continue
    const lane = lanes.get(requestId) ?? []
    lane.push(e)
    lanes.set(requestId, lane)
  }
  return lanes
}

// Pair each version with the shaping lane that produced it (via session id).
export function pairVersions(
  versions: ArtifactVersion[],
  lanes: Map<string, SpineEvent[]>,
): Array<{ version: ArtifactVersion; lane?: SpineEvent[] }> {
  const sessionToLane = new Map<string, SpineEvent[]>()
  for (const lane of lanes.values()) {
    for (const e of lane) {
      const sid = e.data.session_id as string | undefined
      if (sid) sessionToLane.set(sid, lane)
    }
  }
  return versions
    .slice()
    .sort((a, b) => a.version - b.version)
    .map((v) => ({
      version: v,
      lane: sessionToLane.get(v.created_by_session),
    }))
}
