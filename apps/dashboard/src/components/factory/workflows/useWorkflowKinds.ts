import { useMemo } from "react"
import { useQuery } from "@tanstack/react-query"
import {
  Bot,
  CircleDot,
  GitPullRequest,
  Rocket,
  Terminal,
} from "lucide-react"
import { api, type WorkflowKindInfo } from "@/lib/api"
import {
  WORKFLOW_ARTIFACT_KINDS,
  artifactKindMeta as staticArtifactKindMeta,
  type ArtifactKindMeta,
} from "./workflow-meta"

// Icon fallback for registry kinds the static meta list doesn't know about.
// Keeps the card-stack editor visually coherent when a registry-only kind
// (e.g. a tenant-specific Deliverable) shows up in the picker.
const KIND_ICON_FALLBACKS: Record<string, ArtifactKindMeta["icon"]> = {
  Issue: CircleDot,
  PR: GitPullRequest,
  Deployment: Rocket,
  Session: Terminal,
}

function metaFromRegistry(info: WorkflowKindInfo): ArtifactKindMeta {
  const staticMeta = WORKFLOW_ARTIFACT_KINDS.find((k) => k.value === info.id)
  if (staticMeta) {
    return staticMeta
  }
  return {
    value: info.id,
    label: info.description || info.id,
    shortLabel: info.id,
    icon: KIND_ICON_FALLBACKS[info.id] ?? Bot,
  }
}

// Build an alias → canonical-id map from a registry response. Lets us
// resolve a persisted legacy value (e.g. `Spec`, `PullRequest`) to the
// current canonical id without a hardcoded table.
function aliasLookup(kinds: WorkflowKindInfo[]): Record<string, string> {
  const out: Record<string, string> = {}
  for (const k of kinds) {
    for (const alias of k.aliases) {
      out[alias] = k.id
    }
  }
  return out
}

export interface UseWorkflowKindsResult {
  /// Ordered list of kinds to render in pickers. Falls back to the static
  /// set when the registry fetch is still loading or has failed.
  kinds: ArtifactKindMeta[]
  /// Resolve a kind id (possibly a registry alias) to its display meta.
  /// Prefers the runtime-fetched alias map, then falls back to the static
  /// LEGACY_KIND_ALIASES table baked into `artifactKindMeta`.
  metaFor: (value: string) => ArtifactKindMeta
  /// True while the initial fetch is in flight. Callers can use this to
  /// suppress flicker when the static fallback resolves differently from
  /// the registry response.
  isLoading: boolean
  /// True when the fetch failed. UI should silently fall back to the
  /// static list — the workflow builder stays usable offline.
  isError: boolean
}

// Registry-backed workflow artifact kinds (issue #102). Poll-on-load +
// cache for the session; falls back to the static list in `workflow-meta.ts`
// if the fetch fails (offline / dev without stiglab).
export function useWorkflowKinds(): UseWorkflowKindsResult {
  const { data, isLoading, isError } = useQuery({
    queryKey: ["workflow-kinds"],
    queryFn: () => api.listWorkflowKinds(),
    // Registry contents rarely change inside a session. Re-fetch on window
    // focus is enough to pick up a new custom kind without hammering the API.
    staleTime: 5 * 60 * 1000,
    retry: 1,
  })

  return useMemo<UseWorkflowKindsResult>(() => {
    // Fall back to the static list only while the fetch hasn't produced
    // a usable response — i.e. the initial load hasn't resolved yet or
    // the request errored. A successful response with an empty `kinds`
    // array is legitimate server state (registry genuinely has nothing
    // exposed) and should render as empty, not as the static set.
    if (!data || isError) {
      return {
        kinds: WORKFLOW_ARTIFACT_KINDS,
        metaFor: staticArtifactKindMeta,
        isLoading,
        isError,
      }
    }
    const registry = data.kinds
    const kinds = registry.map(metaFromRegistry)
    const byValue = Object.fromEntries(
      kinds.map((k) => [k.value, k]),
    ) as Record<string, ArtifactKindMeta>
    const aliases = aliasLookup(registry)
    const metaFor = (value: string): ArtifactKindMeta => {
      const canonical = aliases[value] ?? value
      return byValue[canonical] ?? staticArtifactKindMeta(value)
    }
    return { kinds, metaFor, isLoading, isError }
  }, [data, isLoading, isError])
}
