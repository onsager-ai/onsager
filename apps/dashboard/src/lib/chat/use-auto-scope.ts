import { useCallback, useEffect, useMemo, useState } from "react"
import { useLocation } from "react-router-dom"
import {
  type ChatScope,
  WORKSPACE_SCOPE,
  scopesEqual,
} from "./scope"
import {
  clearPinnedScope,
  readPinnedScope,
  writePinnedScope,
} from "./pinned-scope"

// Route → scope mapping (ADR 0020 § Contextual auto-scope).
//
// Workflows tab list      → workspace
// Workflow detail         → workflow:{id}
// Cross-workflow runs     → workspace
// Run detail              → run:{runId}
// Verdict detail (future) → verdict:{verdictId}
// Activity / settings /…  → workspace
//
// Verdicts don't have a dedicated route today (they live inside the
// run-detail surface); the verdict scope is reserved for when the
// `/runs/:runId?verdict=…` deep link or a future verdict-detail page
// lands. The hook also reads `?verdict=` off the run-detail surface so
// invocation channels that pass it (#472) get the right scope.
//
// Routes outside `/workspaces/:slug/*` (account settings, picker) fall
// back to the workspace scope of the resolved workspace; if there is
// no workspace at all the hook returns null and the chat should not
// open (callers guard).
export function deriveAutoScope(pathname: string, search: string): ChatScope {
  const params = new URLSearchParams(search)
  const verdictId = params.get("verdict")

  // `/workspaces/:slug/...` — strip the slug, match the remainder.
  const wsMatch = pathname.match(/^\/workspaces\/[^/]+(\/.*)?$/)
  if (!wsMatch) return WORKSPACE_SCOPE
  const rest = wsMatch[1] ?? ""

  // Verdict deep link wins on any sub-route — invocation channels
  // (push notifications, scope-local Ask on verdict cards) carry it
  // explicitly.
  if (verdictId) return { type: "verdict", id: verdictId }

  // Workflow detail. Excludes `/workflows/start` (creation surface,
  // which is workspace-scoped).
  const wfMatch = rest.match(/^\/workflows\/([^/]+)$/)
  if (wfMatch && wfMatch[1] !== "start") {
    return { type: "workflow", id: wfMatch[1] }
  }

  // Run detail.
  const runMatch = rest.match(/^\/runs\/([^/]+)$/)
  if (runMatch) {
    return { type: "run", id: runMatch[1] }
  }

  // Everything else under the workspace — workflows list, runs list,
  // activity, settings, artifact detail, issue detail — is workspace
  // scope per the ADR's "tab switch preserves scope at current level"
  // rule.
  return WORKSPACE_SCOPE
}

export interface AutoScopeState {
  /** Scope derived from the current route. */
  autoScope: ChatScope
  /** User-pinned scope (locks against navigation), or null. */
  pinnedScope: ChatScope | null
  /** What the chat should actually use right now. */
  effectiveScope: ChatScope
  /** True iff `effectiveScope === pinnedScope`. */
  isPinned: boolean
  /** Pin the current effective scope. */
  pin: () => void
  /** Drop the pin so scope follows the route again. */
  unpin: () => void
}

/**
 * Observes route + workspace and returns the chat's effective scope
 * plus the pin controls. Workspace switches drop the pin (pin is
 * per-workspace per the ADR); the workspace switch itself comes
 * through as a `workspaceId` change.
 */
export function useAutoScope(
  userId: string | null,
  workspaceId: string | null,
): AutoScopeState {
  const { pathname, search } = useLocation()

  const autoScope = useMemo(
    () => deriveAutoScope(pathname, search),
    [pathname, search],
  )

  const [pinnedScope, setPinnedScope] = useState<ChatScope | null>(() =>
    readPinnedScope(userId, workspaceId),
  )

  // Workspace switch (or user logout) → drop the pin. Pins are scoped
  // to (user, workspace) in localStorage already, but the in-memory
  // state needs to follow the change.
  useEffect(() => {
    setPinnedScope(readPinnedScope(userId, workspaceId))
  }, [userId, workspaceId])

  const effectiveScope = pinnedScope ?? autoScope

  const pin = useCallback(() => {
    if (!userId || !workspaceId) return
    setPinnedScope(effectiveScope)
    writePinnedScope(userId, workspaceId, effectiveScope)
  }, [userId, workspaceId, effectiveScope])

  const unpin = useCallback(() => {
    if (!userId || !workspaceId) {
      setPinnedScope(null)
      return
    }
    setPinnedScope(null)
    clearPinnedScope(userId, workspaceId)
  }, [userId, workspaceId])

  const isPinned =
    pinnedScope !== null && scopesEqual(pinnedScope, effectiveScope)

  return { autoScope, pinnedScope, effectiveScope, isPinned, pin, unpin }
}
