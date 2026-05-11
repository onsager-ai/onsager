import { Fragment, useState } from "react"
import { Link, useParams } from "react-router-dom"
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query"
import {
  Circle,
  CircleCheck,
  CircleDot,
  CircleX,
  ChevronDown,
  ChevronRight,
} from "lucide-react"
import { api, type SpineEvent, type StageRunStatus } from "@/lib/api"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { ArtifactBadge } from "@/components/factory/workflows/ArtifactBadge"
import { usePageHeader } from "@/components/layout/PageHeader"
import { useActiveWorkspace } from "@/lib/workspace"
import { formatDistanceToNow } from "@/lib/utils"

const STATUS_VARIANT: Record<
  StageRunStatus,
  "default" | "secondary" | "destructive" | "outline"
> = {
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

// Hash persistence keeps deep links to a specific tab working — the
// router doesn't render different routes per tab (single page), so
// `window.location.hash` is the source of truth for which tab is open.
const TAB_VALUES = ["overview", "stages", "artifacts", "verdicts", "events"] as const
type TabValue = (typeof TAB_VALUES)[number]

function readHashTab(): TabValue {
  if (typeof window === "undefined") return "overview"
  const h = window.location.hash.replace(/^#/, "")
  return (TAB_VALUES as readonly string[]).includes(h) ? (h as TabValue) : "overview"
}

export function RunDetailPage() {
  const { runId = "" } = useParams<{ runId: string }>()
  const workspace = useActiveWorkspace()
  const [tab, setTab] = useState<TabValue>(readHashTab)

  const handleTabChange = (next: TabValue) => {
    setTab(next)
    if (typeof window !== "undefined") {
      window.history.replaceState(null, "", `#${next}`)
    }
  }

  const { data, isLoading, isError } = useQuery({
    queryKey: ["run", runId],
    queryFn: () => api.getRun(runId),
    enabled: !!runId,
    refetchInterval: 5000,
  })

  usePageHeader({
    title: data ? (
      <span className="font-mono">{data.run.id.slice(0, 12)}</span>
    ) : (
      "Run"
    ),
    backTo: data
      ? `/workspaces/${workspace.slug}/workflows/${data.workflow.id}`
      : `/workspaces/${workspace.slug}/workflows`,
  })

  if (isLoading) {
    return <p className="py-8 text-center text-muted-foreground">Loading…</p>
  }
  if (isError || !data) {
    return (
      <p className="py-8 text-center text-sm text-destructive">
        Couldn&apos;t load run.
      </p>
    )
  }

  const { run, workflow, stages, sessions } = data
  const stageById = new Map(stages.map((s) => [s.id, s]))

  return (
    <div className="space-y-4 md:space-y-6">
      {/* Desktop-only page header. Mobile uses the global top bar. */}
      <div className="hidden space-y-2 md:block">
        <div className="flex items-center justify-between gap-2">
          <div className="min-w-0">
            <h1 className="truncate text-2xl font-bold tracking-tight font-mono">
              {run.id.slice(0, 12)}
            </h1>
            <p className="truncate text-sm text-muted-foreground">
              <Link
                className="hover:underline"
                to={`/workspaces/${workspace.slug}/workflows/${workflow.id}`}
              >
                {workflow.name}
              </Link>
            </p>
          </div>
          <Badge variant={STATUS_VARIANT[run.status]}>{run.status}</Badge>
        </div>
      </div>

      <Tabs
        value={tab}
        onValueChange={(v) => handleTabChange(v as TabValue)}
      >
        <TabsList variant="line" className="w-full justify-start overflow-x-auto">
          <TabsTrigger value="overview">Overview</TabsTrigger>
          <TabsTrigger value="stages">Stages</TabsTrigger>
          <TabsTrigger value="artifacts">Artifacts</TabsTrigger>
          <TabsTrigger value="verdicts">Verdicts</TabsTrigger>
          <TabsTrigger value="events">Events</TabsTrigger>
        </TabsList>

        <TabsContent value="overview" className="space-y-4 pt-2">
          <Card>
            <CardHeader className="px-4 pb-2 pt-4 md:px-6">
              <CardTitle className="text-base">Overview</CardTitle>
            </CardHeader>
            <CardContent className="space-y-3 px-4 pb-4 md:px-6 text-sm">
              <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
                <Row label="Status">
                  <Badge variant={STATUS_VARIANT[run.status]}>{run.status}</Badge>
                </Row>
                <Row label="Workflow">
                  <Link
                    className="hover:underline"
                    to={`/workspaces/${workspace.slug}/workflows/${workflow.id}`}
                  >
                    {workflow.name}
                  </Link>
                </Row>
                <Row label="Artifact">
                  {run.artifact_id ? (
                    <Link
                      className="font-mono hover:underline"
                      to={`/workspaces/${workspace.slug}/artifacts/${run.artifact_id}`}
                    >
                      {run.artifact_id}
                    </Link>
                  ) : (
                    <span className="text-muted-foreground">—</span>
                  )}
                </Row>
                <Row label="Started">
                  {formatDistanceToNow(run.started_at)}
                </Row>
                <Row label="Updated">
                  {formatDistanceToNow(run.updated_at)}
                </Row>
                <Row label="Sessions">{sessions.length}</Row>
              </div>

              <div className="rounded-md border bg-muted/30 px-3 py-2">
                <div className="mb-1 text-[10px] uppercase tracking-wide text-muted-foreground">
                  Stage progress
                </div>
                <div className="flex items-center gap-1.5">
                  {run.stages.map((s) => {
                    const Icon = STATUS_ICON[s.status]
                    return (
                      <Icon
                        key={s.stage_id}
                        aria-label={s.status}
                        className={`h-4 w-4 ${iconClass(s.status)}`}
                      />
                    )
                  })}
                </div>
              </div>
            </CardContent>
          </Card>
        </TabsContent>

        <TabsContent value="stages" className="space-y-2 pt-2">
          {run.stages.map((rs, i) => {
            const stage = stageById.get(rs.stage_id)
            const stageSessions = sessions.filter(
              // No direct stage_id link on sessions; the agent-session
              // gate typically maps 1:1 with a stage, so list all run
              // sessions on every agent-session panel. Anything more
              // precise needs a `sessions.stage_id` column.
              () => stage?.gate_kind === "agent-session",
            )
            return (
              <StagePanel
                key={rs.stage_id}
                index={i + 1}
                name={stage?.name ?? `Stage ${i + 1}`}
                gateKind={stage?.gate_kind ?? ""}
                status={rs.status}
                sessions={stageSessions}
                workspaceSlug={workspace.slug}
              />
            )
          })}
        </TabsContent>

        <TabsContent value="artifacts" className="space-y-2 pt-2">
          <Card>
            <CardContent className="space-y-2 px-4 py-4 md:px-6 text-sm">
              {run.artifact_id ? (
                <Link
                  className="flex items-center justify-between rounded-md border px-3 py-2 hover:bg-muted/50"
                  to={`/workspaces/${workspace.slug}/artifacts/${run.artifact_id}`}
                >
                  <span className="font-mono text-xs">{run.artifact_id}</span>
                  <ArtifactBadge kind={workflow.stages[0]?.artifact_kind ?? "Issue"} />
                </Link>
              ) : (
                <p className="py-4 text-center text-muted-foreground">
                  No artifacts produced yet.
                </p>
              )}
            </CardContent>
          </Card>
        </TabsContent>

        <TabsContent value="verdicts" className="space-y-2 pt-2">
          <VerdictsCard runId={run.id} workspaceId={workspace.id} />
        </TabsContent>

        <TabsContent value="events" className="space-y-2 pt-2">
          <RunEventsCard runId={run.id} workspaceId={workspace.id} />
        </TabsContent>
      </Tabs>

      {/* Render a hidden grid of all stage statuses so the page still
          shows progress at a glance even when on a non-overview tab. */}
      <div className="sr-only">
        {run.stages.map((s) => (
          <span key={s.stage_id}>{s.status}</span>
        ))}
      </div>
    </div>
  )
}

function Row({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="space-y-0.5">
      <span className="text-muted-foreground text-xs">{label}</span>
      <div>{children}</div>
    </div>
  )
}

function StagePanel({
  index,
  name,
  gateKind,
  status,
  sessions,
  workspaceSlug,
}: {
  index: number
  name: string
  gateKind: string
  status: StageRunStatus
  sessions: Array<{ id: string; state: string; node_id: string }>
  workspaceSlug: string
}) {
  const [open, setOpen] = useState(false)
  const Icon = STATUS_ICON[status]
  return (
    <div className="rounded-md border">
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        className="flex w-full items-center gap-2 px-3 py-2 text-left hover:bg-muted/50"
      >
        {open ? (
          <ChevronDown className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
        ) : (
          <ChevronRight className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
        )}
        <Icon
          aria-label={status}
          className={`h-4 w-4 shrink-0 ${iconClass(status)}`}
        />
        <span className="truncate text-sm font-medium">
          {index}. {name}
        </span>
        {gateKind && (
          <Badge variant="outline" className="ml-auto shrink-0 text-[10px]">
            {gateKind}
          </Badge>
        )}
      </button>
      {open && (
        <div className="space-y-3 border-t bg-muted/20 px-3 py-3 text-sm">
          {sessions.length === 0 ? (
            <p className="text-muted-foreground">No sessions for this stage yet.</p>
          ) : (
            sessions.map((s) => (
              <SessionRow
                key={s.id}
                session={s}
                workspaceSlug={workspaceSlug}
              />
            ))
          )}
        </div>
      )}
    </div>
  )
}

function SessionRow({
  session,
  workspaceSlug,
}: {
  session: { id: string; state: string; node_id: string }
  workspaceSlug: string
}) {
  const queryClient = useQueryClient()
  const cancelMutation = useMutation({
    mutationFn: () => api.cancelSession(session.id),
    onSettled: () =>
      queryClient.invalidateQueries({ queryKey: ["run"] }),
  })

  const terminal = session.state === "done" || session.state === "failed"

  return (
    <div className="flex items-center justify-between gap-2 rounded-md border bg-background px-3 py-2">
      <div className="min-w-0 space-y-0.5">
        <Link
          to={`/workspaces/${workspaceSlug}/sessions/${session.id}`}
          className="font-mono text-xs hover:underline"
        >
          {session.id.slice(0, 12)}
        </Link>
        <div className="flex items-center gap-1.5 text-[10px] text-muted-foreground">
          <Badge variant="outline" className="text-[10px]">
            {session.state}
          </Badge>
          <span className="font-mono">{session.node_id.slice(0, 8)}</span>
        </div>
      </div>
      <Button
        size="sm"
        variant="outline"
        disabled={terminal || cancelMutation.isPending}
        onClick={() => cancelMutation.mutate()}
      >
        {cancelMutation.isPending ? "Cancelling…" : "Cancel"}
      </Button>
    </div>
  )
}

function VerdictsCard({
  runId,
  workspaceId,
}: {
  runId: string
  workspaceId: string
}) {
  // Gate verdicts are `synodic.gate_verdict` events keyed by `gate_id`,
  // but their payloads carry the artifact_id back to the run. The
  // run_id filter joins both shapes server-side.
  const { data, isLoading } = useQuery({
    queryKey: ["run-verdicts", runId],
    queryFn: () =>
      api.getSpineEvents(workspaceId, {
        run_id: runId,
        event_type: "synodic.gate_verdict",
        limit: 100,
      }),
    refetchInterval: 5000,
  })
  const events = data?.events ?? []
  return (
    <Card>
      <CardContent className="space-y-2 px-4 py-4 md:px-6">
        {isLoading ? (
          <p className="py-4 text-center text-sm text-muted-foreground">Loading…</p>
        ) : events.length === 0 ? (
          <p className="py-4 text-center text-sm text-muted-foreground">
            No gate verdicts for this run yet.
          </p>
        ) : (
          events.map((e) => <EventRow key={`${e.id}-${e.stream_id}`} event={e} />)
        )}
      </CardContent>
    </Card>
  )
}

function RunEventsCard({
  runId,
  workspaceId,
}: {
  runId: string
  workspaceId: string
}) {
  const { data, isLoading } = useQuery({
    queryKey: ["run-events", runId],
    queryFn: () =>
      api.getSpineEvents(workspaceId, { run_id: runId, limit: 100 }),
    refetchInterval: 5000,
  })
  const events = data?.events ?? []
  return (
    <Card>
      <CardContent className="space-y-2 px-4 py-4 md:px-6">
        {isLoading ? (
          <p className="py-4 text-center text-sm text-muted-foreground">Loading…</p>
        ) : events.length === 0 ? (
          <p className="py-4 text-center text-sm text-muted-foreground">
            No events for this run yet.
          </p>
        ) : (
          events.map((e) => <EventRow key={`${e.id}-${e.stream_id}`} event={e} />)
        )}
      </CardContent>
    </Card>
  )
}

function EventRow({ event }: { event: SpineEvent }) {
  const [open, setOpen] = useState(false)
  return (
    <div className="rounded-md border">
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        className="flex w-full items-center gap-2 px-3 py-2 text-left hover:bg-muted/50"
      >
        {open ? (
          <ChevronDown className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
        ) : (
          <ChevronRight className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
        )}
        <Badge variant="outline" className="shrink-0 font-mono text-[10px]">
          {event.event_type}
        </Badge>
        <span className="truncate font-mono text-xs text-muted-foreground">
          {event.stream_id}
        </span>
        <span className="ml-auto shrink-0 text-xs text-muted-foreground">
          {new Date(event.created_at).toLocaleString()}
        </span>
      </button>
      {open && (
        <Fragment>
          <pre className="overflow-x-auto whitespace-pre-wrap break-words border-t bg-muted/30 p-3 text-xs text-muted-foreground">
            {JSON.stringify(event.data ?? {}, null, 2)}
          </pre>
        </Fragment>
      )}
    </div>
  )
}
