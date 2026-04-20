import { useQueries, useQuery } from "@tanstack/react-query"
import { api } from "@/lib/api"
import { useAuth } from "@/lib/auth"

export interface SetupProgress {
  authed: boolean
  /**
   * True while the *workspaces* query is still loading. This is the only
   * signal the sidebar's progressive nav disclosure needs — gating nav on
   * the full `loading` would keep the nav visible while `/projects` is
   * still in-flight and cause a flash before hiding.
   */
  workspacesLoading: boolean
  /** True while any of the underlying queries are still loading. */
  loading: boolean
  hasWorkspace: boolean
  hasInstall: boolean
  hasProject: boolean
  complete: boolean
  firstWorkspaceSlug: string | null
}

/**
 * Shared source of truth for workspace onboarding progress. Used by the
 * sidebar's progressive nav disclosure (hide Factory/Governance/Infra until
 * a workspace exists) and the SetupChecklist that surfaces the remaining
 * setup steps. Co-locating the queries keeps the React Query cache honest
 * and avoids double-fetching across callers.
 */
export function useSetupProgress(): SetupProgress {
  const { user, authEnabled } = useAuth()
  const authed = Boolean(authEnabled && user)

  const { data: wsData, isLoading: wsLoading } = useQuery({
    queryKey: ["workspaces"],
    queryFn: api.listWorkspaces,
    enabled: authed,
    staleTime: 30_000,
  })
  const workspaces = wsData?.tenants ?? []
  const firstWs = workspaces[0]

  // Installations live per-tenant; a user may install the App in any of
  // their workspaces. Fan out one query per workspace so the checklist
  // reflects the truth regardless of which workspace got the install.
  // React Query shares the cache with `WorkspaceCard`'s per-id queries.
  const installQueries = useQueries({
    queries: workspaces.map((ws) => ({
      queryKey: ["workspace-installations", ws.id],
      queryFn: () => api.listWorkspaceInstallations(ws.id),
      enabled: authed,
      staleTime: 30_000,
    })),
  })
  const installsLoading = installQueries.some((q) => q.isLoading)
  const anyInstallExists = installQueries.some(
    (q) => (q.data?.installations.length ?? 0) > 0,
  )

  const { data: projectsData, isLoading: projectsLoading } = useQuery({
    queryKey: ["all-projects"],
    queryFn: api.listAllProjects,
    enabled: authed,
    staleTime: 30_000,
  })
  const projects = projectsData?.projects ?? []

  const hasWorkspace = workspaces.length > 0
  // A project implies an install somewhere; otherwise fall back to the
  // aggregated install check across all workspaces.
  const hasInstall = projects.length > 0 || anyInstallExists
  const hasProject = projects.length > 0

  return {
    authed,
    workspacesLoading: authed && wsLoading,
    loading: authed && (wsLoading || installsLoading || projectsLoading),
    hasWorkspace,
    hasInstall,
    hasProject,
    complete: hasWorkspace && hasInstall && hasProject,
    firstWorkspaceSlug: firstWs?.slug ?? null,
  }
}
