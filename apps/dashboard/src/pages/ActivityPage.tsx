import { useMemo } from "react"
import { Link, useSearchParams } from "react-router-dom"
import { useQuery } from "@tanstack/react-query"
import { Activity, ChevronRight } from "lucide-react"
import { api, type SpineEvent } from "@/lib/api"
import { useActiveWorkspace } from "@/lib/workspace"
import { Button } from "@/components/ui/button"
import { Card, CardContent } from "@/components/ui/card"
import { usePageHeader } from "@/components/layout/PageHeader"

// Operator-grain event stream (#454 / ADR 0019). The spine emits a wide
// vocabulary; the Activity feed surfaces the subset operators care about
// as a chronological timeline. Filter chips narrow by source-type; rows
// link out to the relevant detail surface when one exists.

interface SourceTypeChip {
  key: string
  label: string
  // Stream-type prefix or exact match used to derive the chip.
  match: (event: SpineEvent) => boolean
}

const SOURCE_CHIPS: SourceTypeChip[] = [
  { key: "all", label: "All", match: () => true },
  {
    key: "artifact",
    label: "Artifact",
    match: (e) =>
      e.event_type.startsWith("artifact.") ||
      e.event_type.startsWith("forge."),
  },
  {
    key: "run",
    label: "Run",
    match: (e) => e.event_type.startsWith("stiglab."),
  },
  {
    key: "verdict",
    label: "Verdict",
    match: (e) => e.event_type === "verify.verdict",
  },
  {
    key: "trigger",
    label: "Trigger",
    match: (e) =>
      e.event_type.startsWith("trigger.") || e.event_type.startsWith("git."),
  },
  {
    key: "issue",
    label: "Issue",
    match: (e) =>
      e.event_type.includes("issue") || e.event_type.includes("pr"),
  },
]

function readChip(raw: string | null): string {
  if (!raw) return "all"
  return SOURCE_CHIPS.some((c) => c.key === raw) ? raw : "all"
}

export function ActivityPage() {
  usePageHeader({ title: "Activity" })
  const workspace = useActiveWorkspace()
  const [params, setParams] = useSearchParams()

  const chip = readChip(params.get("source"))

  // Pull a generous page of recent events via the path-scoped
  // operator-grain alias (ADR 0019 / spec #465). The backend's
  // `operator_grain=true` filter trims internal subsystem dispatches
  // server-side; chip filtering below narrows the allowlisted stream
  // further on the client (source-type taxonomy). Infinite-scroll is a
  // follow-up; for now, page size 100 covers the typical 7-day
  // operator window.
  const { data, isLoading } = useQuery({
    queryKey: ["activity", workspace.id],
    queryFn: () => api.getActivity(workspace.id, { limit: 100 }),
    refetchInterval: 10_000,
  })

  const visible = useMemo(() => {
    const events = data?.events ?? []
    const filter = SOURCE_CHIPS.find((c) => c.key === chip) ?? SOURCE_CHIPS[0]
    return events.filter((e) => filter.match(e))
  }, [data, chip])

  const setChip = (next: string) => {
    const updated = new URLSearchParams(params)
    if (next === "all") updated.delete("source")
    else updated.set("source", next)
    setParams(updated, { replace: true })
  }

  return (
    <div className="space-y-4 md:space-y-6">
      <div>
        <h1 className="hidden text-2xl font-bold tracking-tight md:block">
          Activity
        </h1>
        <p className="text-sm text-muted-foreground">
          Spine event stream at operator grain.
        </p>
      </div>

      <div className="-mx-1 flex flex-wrap items-center gap-2 px-1">
        {SOURCE_CHIPS.map((c) => {
          const active = chip === c.key
          return (
            <Button
              key={c.key}
              type="button"
              size="sm"
              variant={active ? "default" : "outline"}
              className="h-7 rounded-full px-3 text-xs font-medium"
              onClick={() => setChip(c.key)}
            >
              {c.label}
            </Button>
          )
        })}
      </div>

      {isLoading ? (
        <p className="text-sm text-muted-foreground">Loading…</p>
      ) : visible.length === 0 ? (
        <Card>
          <CardContent className="flex flex-col items-center gap-3 py-10 text-center">
            <div className="flex h-12 w-12 items-center justify-center rounded-full bg-primary/10 text-primary">
              <Activity className="h-6 w-6" />
            </div>
            <div>
              <p className="text-base font-medium">No activity</p>
              <p className="text-sm text-muted-foreground">
                {chip === "all"
                  ? "Nothing has happened in this workspace yet."
                  : "No events match this filter. Try a different chip."}
              </p>
            </div>
          </CardContent>
        </Card>
      ) : (
        <div className="space-y-2">
          {visible.map((e) => (
            <EventRow
              key={`${e.id}`}
              event={e}
              workspaceSlug={workspace.slug}
            />
          ))}
        </div>
      )}
    </div>
  )
}

function EventRow({
  event,
  workspaceSlug,
}: {
  event: SpineEvent
  workspaceSlug: string
}) {
  // Map event → best deep-link. Operator-grain events fall into three
  // shapes (ADR 0019):
  // - Run-keyed: payload carries `artifact_id` (or stream_id is a
  //   `forge:<artifact_id>`). Route to /runs/:id.
  // - Workflow-keyed: payload carries `workflow_id` (trigger.fired,
  //   workflow.manual_triggered). Route to /workflows/:id.
  // - Otherwise unlinked (rendered as a plain row).
  const href = useMemo(() => {
    const data = (event.data ?? {}) as Record<string, unknown>
    const artifactId =
      typeof data.artifact_id === "string" ? data.artifact_id : null
    if (artifactId) {
      return `/workspaces/${workspaceSlug}/runs/${artifactId}`
    }
    // Forge-style stream ids `forge:<artifact_id>` deep-link to the run.
    if (event.stream_id.startsWith("forge:")) {
      const id = event.stream_id.slice("forge:".length)
      if (id) return `/workspaces/${workspaceSlug}/runs/${id}`
    }
    const workflowId =
      typeof data.workflow_id === "string" ? data.workflow_id : null
    if (workflowId) {
      return `/workspaces/${workspaceSlug}/workflows/${workflowId}`
    }
    return null
  }, [event, workspaceSlug])

  const actorLabel = event.actor && event.actor.length > 0 ? event.actor : "system"

  const body = (
    <>
      <div className="flex items-center justify-between gap-2">
        <div className="min-w-0 truncate font-mono text-xs text-muted-foreground">
          {event.event_type}
        </div>
        {href && <ChevronRight className="h-4 w-4 text-muted-foreground" />}
      </div>
      <div className="mt-1 flex items-center justify-between gap-2 text-xs text-muted-foreground">
        <span className="truncate">
          <span className="text-foreground/70">{actorLabel}</span>
          <span aria-hidden="true"> · </span>
          {event.stream_type}/{event.stream_id}
        </span>
        <time dateTime={event.created_at}>
          {new Date(event.created_at).toLocaleString()}
        </time>
      </div>
    </>
  )

  if (href) {
    return (
      <Link
        to={href}
        className="block rounded-md border p-3 hover:bg-muted/50 focus:outline-none focus-visible:ring-2 focus-visible:ring-ring"
      >
        {body}
      </Link>
    )
  }
  return <div className="rounded-md border p-3">{body}</div>
}
