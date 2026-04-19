import { useMemo, useState } from "react"
import { useQuery } from "@tanstack/react-query"
import { useSearchParams } from "react-router-dom"
import { api } from "@/lib/api"
import { Button } from "@/components/ui/button"
import { Skeleton } from "@/components/ui/skeleton"
import { Plus } from "lucide-react"
import { WorkspaceCard } from "@/components/workspaces/WorkspaceCard"
import { WorkspaceOnboarding } from "@/components/workspaces/WorkspaceOnboarding"
import { NewWorkspaceDialog } from "@/components/workspaces/NewWorkspaceDialog"

/**
 * Top-level Workspaces page. Owns the list, the create flow, and the
 * first-run onboarding hero. Replaces the former Settings → Workspaces card.
 */
export function WorkspacesPage() {
  const { data, isLoading } = useQuery({
    queryKey: ["workspaces"],
    queryFn: api.listWorkspaces,
  })
  const workspaces = useMemo(() => data?.tenants ?? [], [data])
  const [createOpen, setCreateOpen] = useState(false)
  const [searchParams, setSearchParams] = useSearchParams()
  const welcome = searchParams.get("welcome") === "1"

  const dismissWelcome = () => {
    if (!welcome) return
    const next = new URLSearchParams(searchParams)
    next.delete("welcome")
    setSearchParams(next, { replace: true })
  }

  const showOnboarding =
    !isLoading && (workspaces.length === 0 || welcome)

  return (
    <div className="space-y-4 md:space-y-6">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div>
          <h1 className="text-xl font-bold tracking-tight md:text-2xl">
            Workspaces
          </h1>
          <p className="max-w-prose text-sm text-muted-foreground">
            A workspace owns GitHub App installations and projects, and scopes
            the sessions your agents run. Installing the App does not
            auto-mirror repos — projects are opt-in per repo.
          </p>
        </div>
        {workspaces.length > 0 && (
          <Button
            onClick={() => {
              setCreateOpen(true)
              dismissWelcome()
            }}
          >
            <Plus className="mr-1 h-4 w-4" />
            New workspace
          </Button>
        )}
      </div>

      {isLoading && (
        <div className="space-y-3">
          <Skeleton className="h-24 w-full" />
          <Skeleton className="h-24 w-full" />
        </div>
      )}

      {showOnboarding && (
        <WorkspaceOnboarding
          onCreate={() => {
            setCreateOpen(true)
            dismissWelcome()
          }}
        />
      )}

      {!isLoading &&
        workspaces.map((ws) => <WorkspaceCard key={ws.id} workspace={ws} />)}

      <NewWorkspaceDialog
        open={createOpen}
        onOpenChange={(open) => {
          setCreateOpen(open)
          if (!open) dismissWelcome()
        }}
      />
    </div>
  )
}
