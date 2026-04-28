import {
  createContext,
  useContext,
  useEffect,
  useMemo,
  type ReactNode,
} from "react"
import { Navigate, useLocation, useParams } from "react-router-dom"
import { useQuery } from "@tanstack/react-query"
import { api, type Workspace } from "@/lib/api"

// Per spec #166: the active workspace is whatever lives at `:workspace`
// in the URL. This provider validates the slug against the user's
// memberships and exposes the resolved row to scoped pages — so each
// page reads `useActiveWorkspace()` instead of pulling from useParams +
// re-querying memberships.

interface WorkspaceContextValue {
  workspace: Workspace
  workspaces: Workspace[]
}

const WorkspaceContext = createContext<WorkspaceContextValue | null>(null)

const LAST_USED_KEY = "onsager.last_used_workspace"

// eslint-disable-next-line react-refresh/only-export-components
export function rememberLastUsedWorkspace(slug: string): void {
  if (typeof window === "undefined") return
  try {
    window.localStorage.setItem(LAST_USED_KEY, slug)
  } catch {
    // localStorage can throw in private mode / quota exhaustion; the
    // worst-case fallback is "user lands on memberships[0]".
  }
}

// eslint-disable-next-line react-refresh/only-export-components
export function readLastUsedWorkspace(): string | null {
  if (typeof window === "undefined") return null
  try {
    return window.localStorage.getItem(LAST_USED_KEY)
  } catch {
    return null
  }
}

/** Fetch the user's workspaces. Auth is always-on as of #193 — the
 * caller is behind ProtectedRoute so the query can fire unconditionally.
 */
// eslint-disable-next-line react-refresh/only-export-components
export function useWorkspacesQuery() {
  return useQuery({
    queryKey: ["workspaces"],
    queryFn: api.listWorkspaces,
    staleTime: 30_000,
  })
}

interface WorkspaceLayoutProps {
  children: ReactNode
}

/**
 * Layout wrapper for `/workspaces/:workspace/*`. Resolves `:workspace`
 * against the user's memberships and either supplies the context or
 * redirects (zero memberships → onboarding; mismatched slug → picker).
 */
export function WorkspaceScope({ children }: WorkspaceLayoutProps) {
  const { workspace: slug } = useParams<{ workspace: string }>()
  const location = useLocation()
  const { data, isLoading } = useWorkspacesQuery()
  const workspaces = useMemo(() => data?.workspaces ?? [], [data])
  const active = useMemo(
    () => workspaces.find((w) => w.slug === slug) ?? null,
    [workspaces, slug],
  )

  useEffect(() => {
    if (active) rememberLastUsedWorkspace(active.slug)
  }, [active])

  if (isLoading) {
    // Tiny shim: render nothing while we resolve. The outer ProtectedRoute
    // already shows the AppShellSkeleton during auth bootstrap, so a flash
    // here is fine (and quicker than a second skeleton).
    return null
  }

  if (workspaces.length === 0) {
    // OnboardingGate handles the welcome redirect; the user landing here
    // directly (deep-link to a scoped path with zero memberships) gets
    // bounced to the picker so they can create one.
    return <Navigate to="/workspaces?welcome=1" replace />
  }

  if (!active) {
    // Slug refers to a workspace the user can't see. Send them to the
    // picker; the search-params hint surfaces the failure inline.
    const params = new URLSearchParams({ unknown: slug ?? "" })
    return (
      <Navigate
        to={{ pathname: "/workspaces", search: `?${params.toString()}` }}
        state={{ from: location.pathname }}
        replace
      />
    )
  }

  return (
    <WorkspaceContext.Provider value={{ workspace: active, workspaces }}>
      {children}
    </WorkspaceContext.Provider>
  )
}

/** Required: callers under `/workspaces/:workspace/*` always have one. */
// eslint-disable-next-line react-refresh/only-export-components
export function useActiveWorkspace(): Workspace {
  const ctx = useContext(WorkspaceContext)
  if (!ctx) {
    throw new Error(
      "useActiveWorkspace must be used inside a /workspaces/:workspace/* route",
    )
  }
  return ctx.workspace
}

/** Optional: callers outside scoped routes (sidebar switcher) read this. */
// eslint-disable-next-line react-refresh/only-export-components
export function useOptionalActiveWorkspace(): Workspace | null {
  const ctx = useContext(WorkspaceContext)
  return ctx?.workspace ?? null
}

/** All workspaces the caller is a member of (memoized list from cache). */
// eslint-disable-next-line react-refresh/only-export-components
export function useMembershipWorkspaces(): Workspace[] {
  // Always run the query (cached after first call) so the switcher can
  // render outside the scoped layout too, e.g. on the `/workspaces`
  // picker page or while bare-path redirects resolve.
  const { data } = useWorkspacesQuery()
  return data?.workspaces ?? []
}
