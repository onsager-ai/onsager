import { useMemo, useState } from "react"
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
import {
  api,
  type RunLinkedSession,
  type SpineEvent,
  type StageRunStatus,
  type WorkflowStage,
} from "@/lib/api"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table"
// Card is intentionally not imported — ADR 0021 inv. 5: detail pages are
// one document of labelled sections, not a stack of Cards.
import { ArtifactBadge } from "@/components/workflows/ArtifactBadge"
import { ChatAskButton } from "@/components/chat/ChatAskButton"
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

// Best-effort stage attribution for a run event. Events carry no
// guaranteed stage key, so we read `stage_index` / `to_stage_index`
// when present and otherwise fold the event onto the final stage — that
// keeps every event reachable under exactly one stage row after the
// tab → stage-timeline collapse (ADR 0021 #489).
function ownerStageIndex(event: SpineEvent, stageCount: number): number {
  const data = (event.data ?? {}) as Record<string, unknown>
  const raw =
    typeof data.stage_index === "number"
      ? data.stage_index
      : typeof data.to_stage_index === "number"
        ? data.to_stage_index
        : null
  if (raw !== null && raw >= 0 && raw < stageCount) return raw
  return Math.max(stageCount - 1, 0)
}

// Payload of a substrate `verify.verdict` event (ADR 0027 / #582):
// `{ type: "verify_verdict", plan_id, node_id, passed, check_results }`.
// Defensive parse — render the pass/fail strip only when the shape holds.
interface VerifyVerdict {
  passed: boolean
  check_results: { name: string; passed: boolean }[]
}

function parseVerifyVerdict(event: SpineEvent): VerifyVerdict | null {
  const data = (event.data ?? {}) as Record<string, unknown>
  if (typeof data.passed !== "boolean") return null
  const raw = Array.isArray(data.check_results) ? data.check_results : []
  const check_results = raw.flatMap((c) => {
    if (
      c !== null &&
      typeof c === "object" &&
      typeof (c as Record<string, unknown>).name === "string" &&
      typeof (c as Record<string, unknown>).passed === "boolean"
    ) {
      return [c as { name: string; passed: boolean }]
    }
    return []
  })
  return { passed: data.passed, check_results }
}

export function RunDetailPage() {
  const { runId = "" } = useParams<{ runId: string }>()
  const workspace = useActiveWorkspace()

  const { data, isLoading, isError } = useQuery({
    queryKey: ["run", runId],
    queryFn: () => api.getRun(runId),
    enabled: !!runId,
    refetchInterval: 5000,
  })

  // Run-scoped events, fetched once and partitioned onto stage rows
  // below. Replaces the former per-attribute Events tab (ADR 0021).
  const eventsQuery = useQuery({
    queryKey: ["run-events", runId, workspace.id],
    queryFn: () =>
      api.getSpineEvents(workspace.id, { run_id: runId, limit: 100 }),
    enabled: !!runId,
    refetchInterval: 5000,
  })
  // Dedicated gate-verdict query. The general events query is capped at
  // 100 rows, so a busy run could push verdicts out of that window and
  // hide them from stage detail; this guarantees the former Verdicts tab
  // content stays reachable regardless of event volume.
  const verdictsQuery = useQuery({
    queryKey: ["run-verdicts", runId, workspace.id],
    queryFn: () =>
      api.getSpineEvents(workspace.id, {
        run_id: runId,
        event_type: "verify.verdict",
        limit: 100,
      }),
    enabled: !!runId,
    refetchInterval: 5000,
  })
  const allEvents = useMemo(() => {
    const byId = new Map<number, SpineEvent>()
    for (const e of eventsQuery.data?.events ?? []) byId.set(e.id, e)
    for (const e of verdictsQuery.data?.events ?? []) byId.set(e.id, e)
    return [...byId.values()]
  }, [eventsQuery.data, verdictsQuery.data])

  // The run's artifact carries its real, observed kind (#568). The stage
  // rows below show this rather than a stage-config prediction — the run
  // produces exactly one artifact (`run.artifact_id`).
  const artifactId = data?.run.artifact_id ?? null
  const artifactQuery = useQuery({
    queryKey: ["artifact", artifactId],
    queryFn: () => api.getArtifact(artifactId as string),
    enabled: !!artifactId,
  })
  const artifactKind = artifactQuery.data?.artifact.kind ?? null

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
  const triggerSource = `${workflow.trigger.repo_owner}/${workflow.trigger.repo_name}${
    workflow.trigger.label ? ` · ${workflow.trigger.label}` : ""
  }`

  return (
    <div className="space-y-4 md:space-y-6">
      {/* Header strip absorbs the former Overview tab (ADR 0021 inv. 4):
          single-value metadata + actions live here, not in a section. */}
      <div className="space-y-3">
        <div className="hidden items-start justify-between gap-2 md:flex">
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
          <div className="flex items-center gap-2">
            <ChatAskButton
              scope={{ type: "run", id: run.id }}
              label="Ask about this run"
            />
            <Badge variant={STATUS_VARIANT[run.status]}>{run.status}</Badge>
          </div>
        </div>

        {/* Mobile context strip — title lives in the global top bar. */}
        <div className="flex items-center justify-between gap-2 md:hidden">
          <Link
            className="min-w-0 truncate text-sm text-muted-foreground hover:underline"
            to={`/workspaces/${workspace.slug}/workflows/${workflow.id}`}
          >
            {workflow.name}
          </Link>
          <Badge variant={STATUS_VARIANT[run.status]}>{run.status}</Badge>
        </div>

        <div className="grid grid-cols-2 gap-x-4 gap-y-2 rounded-md border bg-muted/30 px-3 py-2 text-sm sm:grid-cols-3 lg:grid-cols-5">
          <Meta label="Status">
            <Badge variant={STATUS_VARIANT[run.status]}>{run.status}</Badge>
          </Meta>
          <Meta label="Workflow">
            <Link
              className="truncate hover:underline"
              to={`/workspaces/${workspace.slug}/workflows/${workflow.id}`}
            >
              {workflow.name}
            </Link>
          </Meta>
          <Meta label="Started">{formatDistanceToNow(run.started_at)}</Meta>
          <Meta label="Sessions">{sessions.length}</Meta>
          <Meta label="Trigger">
            <span className="truncate" title={triggerSource}>
              {triggerSource}
            </span>
          </Meta>
        </div>

        <div
          className="flex items-center gap-1.5"
          aria-label="Stage progress"
        >
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

      {/* Stages — the run's only first-class child (ADR 0021 inv. 1).
          Each stage's expanded detail carries the attributes that used
          to be the Artifacts / Verdicts / Events tabs. */}
      <section className="space-y-2">
        <h2 className="text-lg font-semibold tracking-tight">Stages</h2>
        {run.stages.map((rs, i) => {
          const stage = stageById.get(rs.stage_id)
          const stageSessions = sessions.filter(
            // No direct stage_id link on sessions; the agent-session
            // gate maps 1:1 with a stage in practice, so list run
            // sessions on every agent-session panel.
            () => stage?.gate_kind === "agent-session",
          )
          const stageEvents = allEvents.filter(
            (e) => ownerStageIndex(e, run.stages.length) === i,
          )
          return (
            <StagePanel
              key={rs.stage_id}
              index={i + 1}
              stage={stage}
              status={rs.status}
              sessions={stageSessions}
              events={stageEvents}
              artifactId={run.artifact_id}
              artifactKind={artifactKind}
              workspaceSlug={workspace.slug}
              runId={run.id}
            />
          )
        })}
      </section>
    </div>
  )
}

function Meta({
  label,
  children,
}: {
  label: string
  children: React.ReactNode
}) {
  return (
    <div className="min-w-0 space-y-0.5">
      <div className="text-[10px] uppercase tracking-wide text-muted-foreground">
        {label}
      </div>
      <div className="truncate">{children}</div>
    </div>
  )
}

function FieldLabel({ children }: { children: React.ReactNode }) {
  return (
    <div className="text-[10px] uppercase tracking-wide text-muted-foreground">
      {children}
    </div>
  )
}

function StagePanel({
  index,
  stage,
  status,
  sessions,
  events,
  artifactId,
  artifactKind,
  workspaceSlug,
  runId,
}: {
  index: number
  stage: WorkflowStage | undefined
  status: StageRunStatus
  sessions: RunLinkedSession[]
  events: SpineEvent[]
  artifactId: string | null
  artifactKind: string | null
  workspaceSlug: string
  runId: string
}) {
  const [open, setOpen] = useState(false)
  const Icon = STATUS_ICON[status]
  const name = stage?.name ?? `Stage ${index}`
  const gateKind = stage?.gate_kind ?? ""
  const verdicts = events.filter((e) => e.event_type === "verify.verdict")
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
        <div className="space-y-4 border-t bg-muted/20 px-3 py-3 text-sm">
          {/* Output artifact (former Artifacts tab). The kind is the run
              artifact's real, observed kind (#568) — not a stage-config
              prediction. */}
          {artifactId && (
            <div className="space-y-1">
              <FieldLabel>Output</FieldLabel>
              <Link
                to={`/workspaces/${workspaceSlug}/artifacts/${artifactId}`}
                className="flex items-center justify-between rounded-md border bg-background px-3 py-2 hover:bg-muted/50"
              >
                <span className="truncate font-mono text-xs">{artifactId}</span>
                {artifactKind && <ArtifactBadge kind={artifactKind} />}
              </Link>
            </div>
          )}

          {/* Sessions */}
          <div className="space-y-1">
            <FieldLabel>Sessions</FieldLabel>
            {sessions.length === 0 ? (
              <p className="text-muted-foreground">No sessions for this stage.</p>
            ) : (
              <div className="space-y-2">
                {sessions.map((s) => (
                  <SessionRow
                    key={s.id}
                    session={s}
                    workspaceSlug={workspaceSlug}
                  />
                ))}
              </div>
            )}
          </div>

          {/* Verdicts (former Verdicts tab) — substrate Verify verdicts
              (ADR 0027 / #582): `{ passed, check_results }`. */}
          {verdicts.length > 0 && (
            <div className="space-y-1">
              <FieldLabel>Verdicts</FieldLabel>
              <div className="space-y-2">
                {verdicts.map((e) => {
                  const verdict = parseVerifyVerdict(e)
                  return (
                    <div
                      key={`${e.id}`}
                      className="space-y-2 rounded-md border bg-background p-2"
                    >
                      <div className="flex items-center gap-2">
                        {/* SourceTag (#488) */}
                        <Badge
                          variant="outline"
                          className="shrink-0 font-mono text-[10px]"
                        >
                          {e.event_type}
                        </Badge>
                        {verdict && (
                          <Badge
                            variant={verdict.passed ? "default" : "destructive"}
                            className="shrink-0 text-[10px]"
                          >
                            {verdict.passed ? "passed" : "failed"}
                          </Badge>
                        )}
                        <span className="ml-auto shrink-0 text-xs text-muted-foreground">
                          {new Date(e.created_at).toLocaleString()}
                        </span>
                      </div>
                      {verdict && verdict.check_results.length > 0 && (
                        <ul className="space-y-1">
                          {verdict.check_results.map((c, i) => (
                            <li
                              key={`${c.name}-${i}`}
                              className="flex items-center gap-1.5 text-xs"
                            >
                              {c.passed ? (
                                <CircleCheck className="h-3.5 w-3.5 shrink-0 text-green-500" />
                              ) : (
                                <CircleX className="h-3.5 w-3.5 shrink-0 text-destructive" />
                              )}
                              <span className="truncate">{c.name}</span>
                            </li>
                          ))}
                        </ul>
                      )}
                      <div className="flex justify-end">
                        <ChatAskButton
                          scope={{ type: "verdict", id: e.stream_id }}
                          label="Ask about this verdict"
                          variant="ghost"
                        />
                      </div>
                    </div>
                  )
                })}
              </div>
            </div>
          )}

          {/* Events (former Events tab), table form. */}
          <div className="space-y-1">
            <FieldLabel>Events</FieldLabel>
            <EventTable events={events} />
          </div>

          <div className="flex justify-end">
            <ChatAskButton
              scope={{ type: "run", id: runId }}
              label="Ask about this step"
              variant="ghost"
            />
          </div>
        </div>
      )}
    </div>
  )
}

function SessionRow({
  session,
  workspaceSlug,
}: {
  session: RunLinkedSession
  workspaceSlug: string
}) {
  const queryClient = useQueryClient()
  const cancelMutation = useMutation({
    mutationFn: () => api.cancelSession(session.id),
    onSettled: () => queryClient.invalidateQueries({ queryKey: ["run"] }),
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

function EventTable({ events }: { events: SpineEvent[] }) {
  const [showAll, setShowAll] = useState(false)
  if (events.length === 0) {
    return <p className="text-muted-foreground">No events for this stage.</p>
  }
  const sorted = [...events].sort(
    (a, b) => Date.parse(b.created_at) - Date.parse(a.created_at),
  )
  const visible = showAll ? sorted : sorted.slice(0, 20)
  return (
    <div className="space-y-2">
      <div className="overflow-x-auto rounded-md border bg-background">
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>Event</TableHead>
              <TableHead>Source</TableHead>
              <TableHead>Stream</TableHead>
              <TableHead className="text-right">Time</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {visible.map((e) => (
              <TableRow key={`${e.id}`}>
                <TableCell>
                  <Badge variant="outline" className="font-mono text-[10px]">
                    {e.event_type}
                  </Badge>
                </TableCell>
                <TableCell className="text-xs text-muted-foreground">
                  {/* SourceTag (#488) */}
                  {e.actor || "system"}
                </TableCell>
                <TableCell className="max-w-[160px] truncate font-mono text-xs text-muted-foreground">
                  {e.stream_id}
                </TableCell>
                <TableCell className="text-right text-xs text-muted-foreground">
                  {new Date(e.created_at).toLocaleString()}
                </TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      </div>
      {sorted.length > 20 && (
        <Button variant="ghost" size="sm" onClick={() => setShowAll((v) => !v)}>
          {showAll ? "Show less" : `Show all (${sorted.length})`}
        </Button>
      )}
    </div>
  )
}
