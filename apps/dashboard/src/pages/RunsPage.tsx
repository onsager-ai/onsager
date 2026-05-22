import { useMemo } from "react"
import { Link, useSearchParams } from "react-router-dom"
import { useQuery } from "@tanstack/react-query"
import { Activity } from "lucide-react"
import { api, type StageRunStatus, type WorkflowRun } from "@/lib/api"
import { useActiveWorkspace } from "@/lib/workspace"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Card, CardContent } from "@/components/ui/card"
import { usePageHeader } from "@/components/layout/PageHeader"
import { cn } from "@/lib/utils"

// Cross-workflow runs list (#454 / ADR 0019). Each row links to the
// existing flat `/workspaces/:slug/runs/:runId` detail route.
// Filterable by status + workflow id; both are URL-persisted so deep
// links and refreshes preserve the filter.

const STATUS_FILTERS = [
  { key: "all", label: "All" },
  { key: "pending", label: "Pending" },
  { key: "blocked", label: "Blocked" },
  { key: "passed", label: "Passed" },
  { key: "failed", label: "Failed" },
] as const

type StatusFilter = (typeof STATUS_FILTERS)[number]["key"]

const STATUS_VARIANT: Record<
  StageRunStatus,
  "default" | "secondary" | "destructive" | "outline"
> = {
  pending: "outline",
  blocked: "secondary",
  passed: "default",
  failed: "destructive",
}

function readStatusFilter(raw: string | null): StatusFilter {
  if (!raw) return "all"
  return (STATUS_FILTERS.map((f) => f.key) as readonly string[]).includes(raw)
    ? (raw as StatusFilter)
    : "all"
}

export function RunsPage() {
  usePageHeader({ title: "Runs" })
  const workspace = useActiveWorkspace()
  const [params, setParams] = useSearchParams()

  const statusFilter = readStatusFilter(params.get("status"))
  const workflowFilter = params.get("workflow_id") ?? ""

  const { data: runsData, isLoading } = useQuery({
    queryKey: [
      "workspace-runs",
      workspace.id,
      workflowFilter || "*",
    ],
    queryFn: () =>
      api.listWorkspaceRuns(workspace.id, {
        workflowId: workflowFilter || undefined,
        limit: 100,
      }),
  })

  const { data: workflowsData } = useQuery({
    queryKey: ["workflows", workspace.id, "for-runs-filter"],
    queryFn: () => api.listWorkflows(workspace.id),
    staleTime: 30_000,
  })
  const workflowName = useMemo(() => {
    if (!workflowFilter) return null
    return (
      workflowsData?.workflows?.find((w) => w.id === workflowFilter)?.name ??
      null
    )
  }, [workflowFilter, workflowsData])

  const runs = useMemo(() => {
    const all = runsData?.runs ?? []
    if (statusFilter === "all") return all
    return all.filter((r) => r.status === statusFilter)
  }, [runsData, statusFilter])

  const setStatusFilter = (next: StatusFilter) => {
    const updated = new URLSearchParams(params)
    if (next === "all") updated.delete("status")
    else updated.set("status", next)
    setParams(updated, { replace: true })
  }
  const clearWorkflow = () => {
    const updated = new URLSearchParams(params)
    updated.delete("workflow_id")
    setParams(updated, { replace: true })
  }

  return (
    <div className="space-y-4 md:space-y-6">
      <div className="flex items-center justify-between gap-2">
        <div>
          <h1 className="hidden text-2xl font-bold tracking-tight md:block">
            Runs
          </h1>
          <p className="text-sm text-muted-foreground">
            Every artifact flowing through a workflow in this workspace.
          </p>
        </div>
      </div>

      <div className="-mx-1 flex flex-wrap items-center gap-2 px-1">
        {STATUS_FILTERS.map((f) => {
          const active = statusFilter === f.key
          return (
            <Button
              key={f.key}
              type="button"
              size="sm"
              variant={active ? "default" : "outline"}
              className="h-7 rounded-full px-3 text-xs font-medium"
              onClick={() => setStatusFilter(f.key)}
            >
              {f.label}
            </Button>
          )
        })}
        {workflowFilter && (
          <Button
            type="button"
            size="sm"
            variant="secondary"
            className="h-7 rounded-full px-3 text-xs font-medium"
            onClick={clearWorkflow}
            aria-label="Clear workflow filter"
          >
            Workflow: {workflowName ?? workflowFilter.slice(0, 8)} ×
          </Button>
        )}
      </div>

      {isLoading ? (
        <p className="text-sm text-muted-foreground">Loading…</p>
      ) : runs.length === 0 ? (
        <Card>
          <CardContent className="flex flex-col items-center gap-3 py-10 text-center">
            <div className="flex h-12 w-12 items-center justify-center rounded-full bg-primary/10 text-primary">
              <Activity className="h-6 w-6" />
            </div>
            <div>
              <p className="text-base font-medium">No runs match</p>
              <p className="text-sm text-muted-foreground">
                {statusFilter === "all" && !workflowFilter
                  ? "No artifacts have flowed through a workflow in this workspace yet."
                  : "Try a different filter, or clear it to see all runs."}
              </p>
            </div>
          </CardContent>
        </Card>
      ) : (
        <div className="space-y-2">
          {runs.map((r) => (
            <RunRow key={r.id} run={r} workspaceSlug={workspace.slug} />
          ))}
        </div>
      )}
    </div>
  )
}

function RunRow({
  run,
  workspaceSlug,
}: {
  run: WorkflowRun
  workspaceSlug: string
}) {
  const completed = run.stages.filter((s) => s.status === "passed").length
  return (
    <Link
      to={`/workspaces/${workspaceSlug}/runs/${run.id}`}
      className={cn(
        "block rounded-md border p-3 hover:bg-muted/50",
        "focus:outline-none focus-visible:ring-2 focus-visible:ring-ring",
      )}
    >
      <div className="flex items-center justify-between gap-2">
        <div className="min-w-0 truncate font-mono text-sm">
          {run.artifact_id ?? run.id}
        </div>
        <Badge variant={STATUS_VARIANT[run.status]}>{run.status}</Badge>
      </div>
      <div className="mt-1 flex items-center justify-between gap-2 text-xs text-muted-foreground">
        <span>
          {completed} of {run.stages.length} stages passed
        </span>
        <time dateTime={run.updated_at}>
          {new Date(run.updated_at).toLocaleString()}
        </time>
      </div>
    </Link>
  )
}
