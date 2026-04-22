import { Link, useParams } from "react-router-dom"
import { useQuery } from "@tanstack/react-query"
import {
  ArrowLeft,
  ArrowRight,
  Circle,
  CircleCheck,
  CircleDot,
  CircleX,
} from "lucide-react"
import { api, type StageRunStatus, type WorkflowRun } from "@/lib/api"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card"
import { ArtifactBadge } from "@/components/factory/workflows/ArtifactBadge"
import { ArtifactFlowOverview } from "@/components/factory/workflows/ArtifactFlowOverview"
import { outputArtifactKind } from "@/components/factory/workflows/workflow-meta"

const STATUS_VARIANT: Record<StageRunStatus, "default" | "secondary" | "destructive" | "outline"> = {
  pending: "outline",
  blocked: "secondary",
  passed: "default",
  failed: "destructive",
}

const STATUS_ICON: Record<StageRunStatus, typeof Circle> = {
  pending: Circle,
  blocked: CircleDot,
  passed: CircleCheck,
  failed: CircleX,
}

export function WorkflowDetailPage() {
  const { id = "" } = useParams<{ id: string }>()

  const { data, isLoading, isError } = useQuery({
    queryKey: ["workflow", id],
    queryFn: () => api.getWorkflow(id),
    enabled: !!id,
  })
  // Live view of artifacts flowing through stages. The spine bus emits
  // `forge.stage_*` events. Until a push channel (WebSocket/SSE) lands,
  // poll at 5s — matches the rest of the dashboard's fast-refresh cadence
  // without waking the mobile radio every 2s.
  const { data: runsData } = useQuery({
    queryKey: ["workflow-runs", id],
    queryFn: () => api.getWorkflowRuns(id, 50),
    enabled: !!id,
    refetchInterval: 5000,
  })
  const workflow = data?.workflow
  const runs = runsData?.runs ?? []

  if (isLoading) return <p className="text-sm text-muted-foreground">Loading…</p>
  if (isError || !workflow) {
    return (
      <div className="space-y-3">
        <BackLink />
        <p className="text-sm text-destructive">Couldn&apos;t load workflow.</p>
      </div>
    )
  }

  return (
    <div className="space-y-4 md:space-y-6">
      <div className="space-y-2">
        <BackLink />
        <div className="flex items-center justify-between gap-2">
          <div className="min-w-0">
            <h1 className="truncate text-xl font-bold tracking-tight md:text-2xl">
              {workflow.name}
            </h1>
            <p className="truncate text-sm text-muted-foreground">
              {workflow.trigger.repo_owner}/{workflow.trigger.repo_name}
              {workflow.trigger.label ? ` · ${workflow.trigger.label}` : ""}
            </p>
          </div>
          <Badge variant={workflow.status === "active" ? "default" : "outline"}>
            {workflow.status}
          </Badge>
        </div>
      </div>

      <Card>
        <CardHeader className="px-4 pb-2 pt-4 md:px-6">
          <CardTitle className="text-base">Stages</CardTitle>
        </CardHeader>
        <CardContent className="space-y-3 px-4 pb-4 md:px-6">
          <div className="rounded-md border bg-muted/30 px-3 py-2">
            <div className="mb-1 text-[10px] uppercase tracking-wide text-muted-foreground">
              Flow
            </div>
            <ArtifactFlowOverview
              triggerLabel={workflow.trigger.label ?? ""}
              stages={workflow.stages}
            />
          </div>
          {workflow.stages.map((s, i) => {
            const output = outputArtifactKind(s.gate_kind, s.artifact_kind)
            const transforms = output !== s.artifact_kind
            return (
              <div
                key={s.id}
                className="flex items-center justify-between gap-2 rounded-md border px-3 py-2"
              >
                <div className="min-w-0 space-y-1">
                  <div className="truncate text-sm font-medium">
                    {i + 1}. {s.name}
                  </div>
                  <div className="flex flex-wrap items-center gap-1.5">
                    <span className="text-[10px] uppercase tracking-wide text-muted-foreground">
                      {s.gate_kind}
                    </span>
                    <span className="text-muted-foreground/50">·</span>
                    <span className="text-[10px] uppercase tracking-wide text-muted-foreground">
                      in
                    </span>
                    <ArtifactBadge kind={s.artifact_kind} />
                    {transforms && (
                      <>
                        <ArrowRight className="h-3 w-3 text-muted-foreground" />
                        <span className="text-[10px] uppercase tracking-wide text-muted-foreground">
                          out
                        </span>
                        <ArtifactBadge kind={output} />
                      </>
                    )}
                  </div>
                </div>
              </div>
            )
          })}
        </CardContent>
      </Card>

      <Card>
        <CardHeader className="px-4 pb-2 pt-4 md:px-6">
          <CardTitle className="text-base">Live artifacts</CardTitle>
        </CardHeader>
        <CardContent className="space-y-3 px-4 pb-4 md:px-6">
          {runs.length === 0 ? (
            <p className="py-4 text-center text-sm text-muted-foreground">
              No artifacts flowing yet. Tag an issue with the trigger label to
              kick one off.
            </p>
          ) : (
            runs.map((r) => <RunRow key={r.id} run={r} stageIds={workflow.stages.map((s) => s.id)} />)
          )}
        </CardContent>
      </Card>
    </div>
  )
}

function RunRow({ run, stageIds }: { run: WorkflowRun; stageIds: string[] }) {
  const byStage = new Map(run.stages.map((s) => [s.stage_id, s.status]))
  return (
    <div className="space-y-2 rounded-md border p-3">
      <div className="flex items-center justify-between gap-2">
        <div className="min-w-0 text-sm font-mono truncate">
          {run.artifact_id ?? run.id}
        </div>
        <Badge variant={STATUS_VARIANT[run.status]}>{run.status}</Badge>
      </div>
      <div className="flex items-center gap-1">
        {stageIds.map((sid) => {
          const status = byStage.get(sid) ?? "pending"
          const Icon = STATUS_ICON[status]
          return (
            <Icon
              key={sid}
              aria-label={status}
              className={`h-4 w-4 ${iconClass(status)}`}
            />
          )
        })}
      </div>
    </div>
  )
}

function iconClass(status: StageRunStatus): string {
  switch (status) {
    case "passed":
      return "text-green-500"
    case "failed":
      return "text-destructive"
    case "blocked":
      return "text-yellow-500"
    default:
      return "text-muted-foreground"
  }
}

function BackLink() {
  return (
    <Button variant="ghost" size="sm" render={<Link to="/workflows" />}>
      <ArrowLeft className="h-4 w-4" />
      Workflows
    </Button>
  )
}
