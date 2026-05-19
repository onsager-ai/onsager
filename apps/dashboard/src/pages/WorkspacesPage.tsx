import { useMemo, useState } from "react"
import { useQuery } from "@tanstack/react-query"
import { api } from "@/lib/api"
import { Button } from "@/components/ui/button"
import { Skeleton } from "@/components/ui/skeleton"
import { Plus } from "lucide-react"
import { WorkspaceCard } from "@/components/workspaces/WorkspaceCard"
import { NewWorkspaceDialog } from "@/components/workspaces/NewWorkspaceDialog"
import { usePageHeader } from "@/components/layout/PageHeader"

/**
 * Top-level Workspaces page. Lists the user's workspaces and exposes the
 * explicit create flow. The first-run hero (`WorkspaceOnboarding`) and the
 * `welcome=1` URL param were demolished in spec #403 — the FTUE entry is
 * now `/chat`, and binding lifts workspace creation into a single dialog.
 */
export function WorkspacesPage() {
  usePageHeader({ title: "Workspaces" })
  const { data, isLoading } = useQuery({
    queryKey: ["workspaces"],
    queryFn: api.listWorkspaces,
  })
  const workspaces = useMemo(() => data?.workspaces ?? [], [data])
  const [createOpen, setCreateOpen] = useState(false)

  return (
    <div className="space-y-4 md:space-y-6">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div>
          <h1 className="hidden text-2xl font-bold tracking-tight md:block">
            Workspaces
          </h1>
          <p className="max-w-prose text-sm text-muted-foreground">
            A workspace owns GitHub App installations and projects, and scopes
            the sessions your agents run. Installing the App does not
            auto-mirror repos — projects are opt-in per repo.
          </p>
        </div>
        <Button onClick={() => setCreateOpen(true)}>
          <Plus className="mr-1 h-4 w-4" />
          New workspace
        </Button>
      </div>

      {isLoading && (
        <div className="space-y-3">
          <Skeleton className="h-24 w-full" />
          <Skeleton className="h-24 w-full" />
        </div>
      )}

      {!isLoading &&
        workspaces.map((ws) => <WorkspaceCard key={ws.id} workspace={ws} />)}

      <NewWorkspaceDialog open={createOpen} onOpenChange={setCreateOpen} />
    </div>
  )
}
