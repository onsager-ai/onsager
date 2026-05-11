import { useMemo, useState } from "react"
import { Link } from "react-router-dom"
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query"
import { CircleDot, Loader2, XCircle } from "lucide-react"
import { api, ApiError, type WorkflowRun } from "@/lib/api"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card"
import { useActiveWorkspace } from "@/lib/workspace"

// In-flight = the run hasn't reached a terminal stage status. `pending` and
// `blocked` are both still moving (or waiting on a gate); `passed`/`failed`
// are done. Cancel maps to abortArtifact on the run's artifact — there is
// no per-run abort endpoint today, and the artifact is the durable unit.
function isInFlight(run: WorkflowRun): boolean {
  return run.status === "pending" || run.status === "blocked"
}

interface ActiveRunsBannerProps {
  // When given, scopes the banner to one workflow. Omit on the Workflows
  // landing to render every in-flight run in the workspace.
  workflowId?: string
  // Required so the landing variant can fan out across workflows. The
  // detail variant passes it for consistency with the listing variant.
  workspaceId: string
  // Optional title override — the landing wants "Active runs", the detail
  // page wants "In-flight" inline above the runs list.
  title?: string
}

export function ActiveRunsBanner({
  workflowId,
  workspaceId,
  title = "Active runs",
}: ActiveRunsBannerProps) {
  const workspace = useActiveWorkspace()

  const scopedQuery = useQuery({
    queryKey: ["workflow-runs", workflowId],
    queryFn: () => api.getWorkflowRuns(workflowId!, 50),
    enabled: !!workflowId,
    refetchInterval: 5000,
  })

  const workspaceQuery = useQuery({
    queryKey: ["active-runs", workspaceId],
    queryFn: async () => {
      const { workflows } = await api.listWorkflows(workspaceId)
      const lists = await Promise.all(
        workflows.map((w) => api.getWorkflowRuns(w.id, 20).then((r) => r.runs)),
      )
      return lists.flat()
    },
    enabled: !workflowId,
    refetchInterval: 5000,
  })

  const runs = useMemo(() => {
    const all = workflowId
      ? (scopedQuery.data?.runs ?? [])
      : (workspaceQuery.data ?? [])
    return all.filter(isInFlight)
  }, [workflowId, scopedQuery.data, workspaceQuery.data])

  if (runs.length === 0) return null

  return (
    <Card className="border-primary/40 bg-primary/5">
      <CardHeader className="px-4 pb-2 pt-4 md:px-6">
        <CardTitle className="flex items-center gap-2 text-base">
          <CircleDot className="h-4 w-4 animate-pulse text-primary" />
          {title}
          <Badge variant="outline" className="ml-1">
            {runs.length}
          </Badge>
        </CardTitle>
      </CardHeader>
      <CardContent className="space-y-2 px-4 pb-4 md:px-6">
        {runs.map((r) => (
          <ActiveRunRow
            key={r.id}
            run={r}
            workspaceSlug={workspace.slug}
          />
        ))}
      </CardContent>
    </Card>
  )
}

function ActiveRunRow({
  run,
  workspaceSlug,
}: {
  run: WorkflowRun
  workspaceSlug: string
}) {
  const qc = useQueryClient()
  const [error, setError] = useState<string | null>(null)

  const cancel = useMutation({
    mutationFn: () => {
      if (!run.artifact_id) {
        throw new ApiError("Run has no artifact to cancel.", 400)
      }
      return api.abortArtifact(run.artifact_id, {
        reason: "cancelled from active-runs banner",
        actor: "dashboard",
      })
    },
    onSuccess: () => {
      setError(null)
      qc.invalidateQueries({ queryKey: ["workflow-runs", run.workflow_id] })
      qc.invalidateQueries({ queryKey: ["active-runs"] })
    },
    onError: (err) => {
      const message =
        err instanceof ApiError
          ? err.message
          : err instanceof Error
            ? err.message
            : "unknown error"
      setError(message)
    },
  })

  return (
    <div className="space-y-1">
      <div className="flex items-center gap-2 rounded-md border bg-background px-3 py-2">
        <Badge variant="secondary" className="shrink-0">
          {run.status}
        </Badge>
        {run.artifact_id ? (
          <Link
            to={`/workspaces/${workspaceSlug}/artifacts/${run.artifact_id}`}
            className="min-w-0 flex-1 truncate font-mono text-xs hover:underline"
          >
            {run.artifact_id}
          </Link>
        ) : (
          <span className="min-w-0 flex-1 truncate font-mono text-xs text-muted-foreground">
            {run.id}
          </span>
        )}
        <span className="hidden shrink-0 text-xs text-muted-foreground sm:inline">
          {new Date(run.updated_at).toLocaleTimeString()}
        </span>
        <Button
          variant="ghost"
          size="sm"
          disabled={!run.artifact_id || cancel.isPending}
          onClick={() => cancel.mutate()}
          aria-label="Cancel run"
        >
          {cancel.isPending ? (
            <Loader2 className="h-3.5 w-3.5 animate-spin" />
          ) : (
            <XCircle className="h-3.5 w-3.5" />
          )}
          Cancel
        </Button>
      </div>
      {error && (
        <p className="px-3 text-xs text-destructive">{error}</p>
      )}
    </div>
  )
}
