import { Link } from "react-router-dom"
import { useQuery } from "@tanstack/react-query"
import { Package } from "lucide-react"
import { api } from "@/lib/api"
import { Badge } from "@/components/ui/badge"
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card"
import { ArtifactBadge } from "./ArtifactBadge"
import { useActiveWorkspace } from "@/lib/workspace"

// Workflow-scoped artifact list from the dedicated backend route (#302).
// Replaces the per-run artifact fan-out we used to do client-side; one
// request, server-side filter, and we can poll without scaling the
// request count with the number of runs.
export function WorkflowArtifactsTab({ workflowId }: { workflowId: string }) {
  const workspace = useActiveWorkspace()
  const { data, isLoading } = useQuery({
    queryKey: ["workflow-artifacts", workflowId],
    queryFn: () => api.getWorkflowArtifacts(workflowId),
    refetchInterval: 30000,
    refetchIntervalInBackground: false,
  })

  const artifacts = data?.artifacts ?? []

  if (!isLoading && artifacts.length === 0) {
    return <EmptyState />
  }

  return (
    <Card>
      <CardHeader className="px-4 pb-2 pt-4 md:px-6">
        <CardTitle className="text-base">Artifacts</CardTitle>
      </CardHeader>
      <CardContent className="space-y-2 px-4 pb-4 md:px-6">
        {isLoading && artifacts.length === 0 ? (
          <p className="py-4 text-center text-sm text-muted-foreground">
            Loading…
          </p>
        ) : (
          artifacts.map((artifact) => (
            <Link
              key={artifact.id}
              to={`/workspaces/${workspace.slug}/artifacts/${artifact.id}`}
              className="flex items-center gap-2 rounded-md border px-3 py-2 hover:bg-muted/50"
            >
              <ArtifactBadge kind={artifact.kind} />
              <span className="min-w-0 flex-1 truncate text-sm">
                {artifact.name ?? artifact.id}
              </span>
              <Badge
                variant={artifact.state === "released" ? "default" : "outline"}
                className="shrink-0"
              >
                {artifact.state}
              </Badge>
              <span className="hidden shrink-0 text-xs text-muted-foreground sm:inline">
                v{artifact.current_version}
              </span>
            </Link>
          ))
        )}
      </CardContent>
    </Card>
  )
}

function EmptyState() {
  return (
    <Card>
      <CardContent className="flex flex-col items-center gap-3 py-10 text-center">
        <div className="flex h-12 w-12 items-center justify-center rounded-full bg-muted text-muted-foreground">
          <Package className="h-6 w-6" />
        </div>
        <div className="space-y-1">
          <p className="text-base font-medium">No artifacts yet</p>
          <p className="text-sm text-muted-foreground">
            Artifacts appear here once this workflow produces them.
          </p>
        </div>
      </CardContent>
    </Card>
  )
}
