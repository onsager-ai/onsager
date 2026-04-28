import { useMemo } from "react"
import { Link } from "react-router-dom"
import { useQueries } from "@tanstack/react-query"
import { api, type WorkflowRun } from "@/lib/api"
import { Badge } from "@/components/ui/badge"
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card"
import { useActiveWorkspace } from "@/lib/workspace"

// Sessions that shaped this workflow's artifacts. The durable edge lives
// in the `vertical_lineage` table (artifact_id, version, session_id),
// which forge writes on every session completion linked to an artifact.
// The `stiglab.session_completed` event does not carry artifact_id in
// its current emission path, so we read lineage directly via the
// artifact-detail endpoint rather than filtering events client-side.
export function WorkflowSessionsCard({ runs }: { runs: WorkflowRun[] }) {
  const workspace = useActiveWorkspace()
  const artifactIds = useMemo(
    () =>
      Array.from(
        new Set(runs.map((r) => r.artifact_id).filter((id): id is string => !!id)),
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

  const rows = useMemo(() => {
    const out: {
      session_id: string
      artifact_id: string
      version: number
      recorded_at: string
    }[] = []
    for (let i = 0; i < queries.length; i++) {
      const artifact_id = artifactIds[i]
      const detail = queries[i].data?.artifact
      if (!detail) continue
      for (const lineage of detail.vertical_lineage ?? []) {
        out.push({
          session_id: lineage.session_id,
          artifact_id,
          version: lineage.version,
          recorded_at: lineage.recorded_at,
        })
      }
    }
    out.sort((a, b) => (a.recorded_at < b.recorded_at ? 1 : -1))
    return out
  }, [queries, artifactIds])

  const loading = queries.some((q) => q.isLoading)

  return (
    <Card>
      <CardHeader className="px-4 pb-2 pt-4 md:px-6">
        <CardTitle className="text-base">Sessions</CardTitle>
      </CardHeader>
      <CardContent className="space-y-2 px-4 pb-4 md:px-6">
        {artifactIds.length === 0 ? (
          <p className="py-4 text-center text-sm text-muted-foreground">
            No artifacts yet — sessions appear once a trigger fires and the
            first stage dispatches one.
          </p>
        ) : loading ? (
          <p className="py-4 text-center text-sm text-muted-foreground">Loading…</p>
        ) : rows.length === 0 ? (
          <p className="py-4 text-center text-sm text-muted-foreground">
            No sessions recorded against this workflow's artifacts yet.
          </p>
        ) : (
          rows.map((r) => (
            <Link
              key={`${r.artifact_id}:${r.session_id}:${r.version}`}
              to={`/workspaces/${workspace.slug}/sessions/${r.session_id}`}
              className="flex items-center gap-2 rounded-md border px-3 py-2 hover:bg-muted/50"
            >
              <Badge variant="outline" className="shrink-0 font-mono text-[10px]">
                v{r.version}
              </Badge>
              <span className="truncate font-mono text-xs">
                {r.session_id.slice(0, 12)}
              </span>
              <span className="truncate font-mono text-[10px] text-muted-foreground">
                → {r.artifact_id}
              </span>
              <span className="ml-auto shrink-0 text-xs text-muted-foreground">
                {new Date(r.recorded_at).toLocaleString()}
              </span>
            </Link>
          ))
        )}
      </CardContent>
    </Card>
  )
}
