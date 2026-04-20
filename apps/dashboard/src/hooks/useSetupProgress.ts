import { useQuery } from "@tanstack/react-query"
import { api } from "@/lib/api"
import { useAuth } from "@/lib/auth"

export interface SetupProgress {
  authed: boolean
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
 * and avoids double-fetching across the two callers.
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

  const { data: installsData, isLoading: installsLoading } = useQuery({
    queryKey: ["workspace-installations", firstWs?.id],
    queryFn: () => api.listWorkspaceInstallations(firstWs!.id),
    enabled: authed && !!firstWs,
    staleTime: 30_000,
  })
  const installs = installsData?.installations ?? []

  const { data: projectsData, isLoading: projectsLoading } = useQuery({
    queryKey: ["all-projects"],
    queryFn: api.listAllProjects,
    enabled: authed,
    staleTime: 30_000,
  })
  const projects = projectsData?.projects ?? []

  const hasWorkspace = workspaces.length > 0
  // A project implies an install somewhere, even if not in the first workspace.
  const hasInstall = installs.length > 0 || projects.length > 0
  const hasProject = projects.length > 0

  return {
    authed,
    loading: authed && (wsLoading || installsLoading || projectsLoading),
    hasWorkspace,
    hasInstall,
    hasProject,
    complete: hasWorkspace && hasInstall && hasProject,
    firstWorkspaceSlug: firstWs?.slug ?? null,
  }
}
