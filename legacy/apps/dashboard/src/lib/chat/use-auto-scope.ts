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

// Route → default chat scope (ADR 0020 § Contextual auto-scope,
// *narrowed* by ADR 0025 § Chat's scope narrows to two jobs, spec #514).
//
// ADR 0025 names chat's two jobs: **design** (authoring/drafting, which
// happens at the workspace level — the CAD bench) and **maintenance**
// (incident handling on a specific run — scope=run). The default scope a
// bare route resolves to is therefore one of exactly two kinds:
//
//   Run detail              → run:{runId}      (maintenance)
//   Everything else          → workspace        (design)
//
// A workflow detail page no longer auto-scopes to `workflow:{id}` — a
// workflow is *designed* at the workspace level, not maintained from its
// detail route, so the route default folds into the design (workspace)
// surface. The `workflow` scope still exists for the explicit
// `ChatAskButton` override ("Ask about this workflow"); it just isn't a
// route *default* any more. That keeps the resolver to design +
// maintenance only.
//
// `?verdict=` is a maintenance sub-scope of a run, carried explicitly by
// invocation channels (push notifications, scope-local Ask on verdict
// cards, #472) — it is an override the resolver honours, never a bare
// route default.
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

  // Verdict deep link wins on any sub-route — a maintenance sub-scope
  // carried explicitly by invocation channels.
  if (verdictId) return { type: "verdict", id: verdictId }

  // Maintenance — run detail is the one route that scopes to a run.
  const runMatch = rest.match(/^\/runs\/([^/]+)$/)
  if (runMatch) {
    return { type: "run", id: runMatch[1] }
  }

  // Design — everything else under the workspace (workflows list +
  // detail, runs list, plans, activity, settings, artifact detail)
  // resolves to the workspace authoring thread.
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
 *
 * `scopeOverride` is a one-shot from `useChatUi.open({ scope })` —
 * an invocation channel (push deep link, scope-local Ask, top bell)
 * pinned a specific scope at open time. It outranks both the
 * route-derived auto scope and the user's pin until the chat closes.
 */
export function useAutoScope(
  userId: string | null,
  workspaceId: string | null,
  scopeOverride?: ChatScope | null,
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

  const effectiveScope = scopeOverride ?? pinnedScope ?? autoScope

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
