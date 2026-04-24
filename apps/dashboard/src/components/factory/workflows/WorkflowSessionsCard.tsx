import { useMemo } from "react"
import { Link } from "react-router-dom"
import { useQuery } from "@tanstack/react-query"
import { api, type WorkflowRun } from "@/lib/api"
import { Badge } from "@/components/ui/badge"
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card"

// Sessions linked to this workflow's artifacts. Derived from
// `stiglab.session_completed` spine events (the only place the
// session<->artifact edge is recorded on the wire). Running sessions
// aren't yet surfaced here — they'd need a separate query that joins the
// live sessions table to the artifact graph.
export function WorkflowSessionsCard({ runs }: { runs: WorkflowRun[] }) {
  const artifactIds = useMemo(
    () =>
      new Set(runs.map((r) => r.artifact_id).filter((id): id is string => !!id)),
    [runs],
  )

  const { data, isLoading } = useQuery({
    queryKey: ["workflow-sessions", [...artifactIds].sort()],
    queryFn: () => api.getSessionSpend(100),
    refetchInterval: 5000,
    enabled: artifactIds.size > 0,
  })

  const rows = useMemo(() => {
    if (!data) return []
    return data.filter((r) => r.artifact_id && artifactIds.has(r.artifact_id))
  }, [data, artifactIds])

  return (
    <Card>
      <CardHeader className="px-4 pb-2 pt-4 md:px-6">
        <CardTitle className="text-base">Sessions</CardTitle>
      </CardHeader>
      <CardContent className="space-y-2 px-4 pb-4 md:px-6">
        {artifactIds.size === 0 ? (
          <p className="py-4 text-center text-sm text-muted-foreground">
            No artifacts yet — sessions appear once a trigger fires and the
            first stage dispatches one.
          </p>
        ) : isLoading ? (
          <p className="py-4 text-center text-sm text-muted-foreground">Loading…</p>
        ) : rows.length === 0 ? (
          <p className="py-4 text-center text-sm text-muted-foreground">
            No completed sessions for this workflow's artifacts yet.
          </p>
        ) : (
          rows.map((r) => (
            <Link
              key={r.id}
              to={`/sessions/${r.session_id}`}
              className="flex items-center gap-2 rounded-md border px-3 py-2 hover:bg-muted/50"
            >
              <Badge variant="outline" className="shrink-0 font-mono text-[10px]">
                session
              </Badge>
              <span className="truncate font-mono text-xs">
                {r.session_id.slice(0, 12)}
              </span>
              {r.artifact_id && (
                <span className="truncate font-mono text-[10px] text-muted-foreground">
                  → {r.artifact_id}
                </span>
              )}
              <span className="ml-auto shrink-0 text-xs text-muted-foreground">
                {formatDuration(r.duration_ms)} ·{" "}
                {new Date(r.created_at).toLocaleString()}
              </span>
            </Link>
          ))
        )}
      </CardContent>
    </Card>
  )
}

function formatDuration(ms: number): string {
  if (ms <= 0) return "—"
  const s = Math.round(ms / 1000)
  if (s < 60) return `${s}s`
  const m = Math.floor(s / 60)
  const rem = s % 60
  return rem === 0 ? `${m}m` : `${m}m${rem}s`
}
