import { createContext, useContext, type ReactNode } from 'react'
import { useQuery } from '@tanstack/react-query'
import { api, type Workspace } from './api'

const WorkspaceContext = createContext<Workspace | null>(null)

// eslint-disable-next-line react-refresh/only-export-components
export function useWorkspace(): Workspace {
  const ws = useContext(WorkspaceContext)
  if (!ws) throw new Error('useWorkspace outside provider')
  return ws
}

/**
 * v2 is effectively single-workspace: every user has a personal
 * workspace (dev seed for dev-login, minted on first OAuth login
 * otherwise), so the provider resolves the first membership and blocks
 * until it exists. A real workspace switcher returns when
 * multi-workspace is a feature.
 */
export function WorkspaceProvider({ children }: { children: ReactNode }) {
  const { data, isLoading } = useQuery({
    queryKey: ['workspaces'],
    queryFn: api.listWorkspaces,
  })
  if (isLoading) {
    return (
      <div className="flex min-h-screen items-center justify-center text-muted-foreground">
        Loading...
      </div>
    )
  }
  const ws = data?.workspaces[0]
  if (!ws) {
    return (
      <div className="flex min-h-screen items-center justify-center text-muted-foreground">
        No workspace available for this account.
      </div>
    )
  }
  return <WorkspaceContext.Provider value={ws}>{children}</WorkspaceContext.Provider>
}
