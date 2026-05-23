import { useSyncExternalStore } from "react"
import type { ChatScope } from "./scope"
import { scopeKey } from "./scope"

// HITL count seam (ADR 0020 § FAB three-state behaviour, spec #472).
//
// The FAB's Active/Muted decision and the bell's badge both need to
// answer "does this scope have a pending HITL?". The actual HITL
// stream — agent-requested approvals queued through the portal MCP
// elicitation flow (ADR 0008) — does not yet emit on the spine as a
// first-class artifact, so this module is the documented seam where
// the future aggregator wires in. Today both hooks return 0; once
// the elicitation stream lands, swap the implementation here without
// touching the FAB / bell / Queue surfaces.
//
// The dispatch helper `dispatchScopeHitlCountChange` is exported so a
// future SSE/WebSocket consumer can fan in updates by scope key.

type CountListener = (n: number) => void

const scopeCounts = new Map<string, number>()
const scopeListeners = new Map<string, Set<CountListener>>()

/** Mutable seam — call from the future elicitation listener. */
export function dispatchScopeHitlCountChange(scope: ChatScope, n: number) {
  const key = scopeKey(scope)
  scopeCounts.set(key, n)
  const subs = scopeListeners.get(key)
  if (subs) for (const fn of subs) fn(n)
}

function subscribe(key: string, fn: CountListener): () => void {
  let subs = scopeListeners.get(key)
  if (!subs) {
    subs = new Set()
    scopeListeners.set(key, subs)
  }
  subs.add(fn)
  return () => {
    subs!.delete(fn)
    if (subs!.size === 0) scopeListeners.delete(key)
  }
}

/** Count of pending HITL items in the given scope. */
export function useHitlCount(scope: ChatScope): number {
  const key = scopeKey(scope)
  // useSyncExternalStore handles the external-store dance — no
  // setState-in-effect cascade, and rerenders fire on store change.
  return useSyncExternalStore(
    (notify) => subscribe(key, () => notify()),
    () => scopeCounts.get(key) ?? 0,
    () => 0,
  )
}

/**
 * Workspace-wide pending HITL count — the bell's badge. Today this is
 * just the workspace-scope count; once finer-scoped HITLs land, this
 * should sum across all scopes for the user/workspace.
 */
export function useWorkspaceHitlCount(): number {
  return useHitlCount({ type: "workspace", id: null })
}
