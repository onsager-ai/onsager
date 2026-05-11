import { useMemo } from "react"
import { Link } from "react-router-dom"
import { useQueries } from "@tanstack/react-query"
import { Package } from "lucide-react"
import { api, type WorkflowRun } from "@/lib/api"
import { Badge } from "@/components/ui/badge"
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card"
import { ArtifactBadge } from "./ArtifactBadge"
import { useActiveWorkspace } from "@/lib/workspace"

// Derived from the runs list: every run produces (at most) one artifact, so
// the unique set of artifact ids on this workflow's runs is the artifact
// inventory for the workflow. PR 2b replaces this with a dedicated backend
// route; this PR keeps the surface shape stable so the swap is a data-source
// change only.
export function WorkflowArtifactsTab({ runs }: { runs: WorkflowRun[] }) {
  const workspace = useActiveWorkspace()
  const artifactIds = useMemo(
    () =>
      Array.from(
        new Set(
          runs
            .map((r) => r.artifact_id)
            .filter((id): id is string => !!id),
        ),
      ),
    [runs],
  )

  const queries = useQueries({
    queries: artifactIds.map((id) => ({
      queryKey: ["artifact", id],
      queryFn: () => api.getArtifact(id),
      refetchInterval: 5000,
    })),
  })

  const loading = queries.some((q) => q.isLoading)
  const items = useMemo(
    () =>
      queries
        .map((q, i) => ({ id: artifactIds[i], artifact: q.data?.artifact }))
        .filter((row) => !!row.artifact),
    [queries, artifactIds],
  )

  if (artifactIds.length === 0) {
    return <EmptyState />
  }

  return (
    <Card>
      <CardHeader className="px-4 pb-2 pt-4 md:px-6">
        <CardTitle className="text-base">Artifacts</CardTitle>
      </CardHeader>
      <CardContent className="space-y-2 px-4 pb-4 md:px-6">
        {loading && items.length === 0 ? (
          <p className="py-4 text-center text-sm text-muted-foreground">
            Loading…
          </p>
        ) : (
          items.map(({ id, artifact }) =>
            artifact ? (
              <Link
                key={id}
                to={`/workspaces/${workspace.slug}/artifacts/${id}`}
                className="flex items-center gap-2 rounded-md border px-3 py-2 hover:bg-muted/50"
              >
                <ArtifactBadge kind={artifact.kind} />
                <span className="min-w-0 flex-1 truncate text-sm">
                  {artifact.name ?? id}
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
            ) : null,
          )
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
