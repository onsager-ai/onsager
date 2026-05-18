// Build-info descriptor (`GET /api/build-info`, spec #398).
//
// The FTUE OSS banner on the workspace-less `/chat` entry only renders
// when `is_oss` is true. Fetched once per session (no auth required) and
// cached at module scope; failures default to `is_oss: false` so a
// down portal doesn't accidentally flash the OSS banner on Cloud.

import { useEffect, useState } from "react"

import { API_BASE } from "@/lib/api/client"

export interface BuildInfo {
  is_oss: boolean
  version: string
}

const FALLBACK: BuildInfo = { is_oss: false, version: "unknown" }

let cached: BuildInfo | null = null
let inflight: Promise<BuildInfo> | null = null

export async function fetchBuildInfo(): Promise<BuildInfo> {
  if (cached) return cached
  if (inflight) return inflight
  inflight = (async () => {
    try {
      const res = await fetch(`${API_BASE}/build-info`)
      if (!res.ok) return FALLBACK
      const json = (await res.json()) as Partial<BuildInfo>
      const info: BuildInfo = {
        is_oss: typeof json.is_oss === "boolean" ? json.is_oss : false,
        version: typeof json.version === "string" ? json.version : "unknown",
      }
      cached = info
      return info
    } catch {
      return FALLBACK
    } finally {
      inflight = null
    }
  })()
  return inflight
}

/** Read the cached build-info (or trigger a fetch on first use). */
export function useBuildInfo(): BuildInfo | null {
  // Lazy-init from the module-scope cache so a remount in the same
  // session doesn't refetch. The async fetch only setStates from a
  // callback (not in an effect body) per React's "set-state-in-effect"
  // guidance — the effect is a no-op once `cached` is populated.
  const [info, setInfo] = useState<BuildInfo | null>(() => cached)
  useEffect(() => {
    if (cached) return
    let cancelled = false
    fetchBuildInfo().then((b) => {
      if (!cancelled) setInfo(b)
    })
    return () => {
      cancelled = true
    }
  }, [])
  return info
}
