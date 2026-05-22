import { Fragment, useMemo, useState } from "react"
import { useQueries } from "@tanstack/react-query"
import { ChevronDown, ChevronRight } from "lucide-react"
import { api, type SpineEvent, type WorkflowRun } from "@/lib/api"
import { Badge } from "@/components/ui/badge"
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card"
import { useActiveWorkspace } from "@/lib/workspace"

// Events scoped to a single workflow: one `trigger.fired` stream keyed by
// workflow_id plus a per-artifact stream (`workflow:{artifact_id}`) for
// stage.entered / stage.gate_* / stage.advanced. Fans out into N+1 small
// queries rather than one over-wide query so we only pull the events that
// actually belong to this workflow.
export function WorkflowEventsCard({
  workflowId,
  runs,
}: {
  workflowId: string
  runs: WorkflowRun[]
}) {
  const artifactIds = useMemo(
    () =>
      Array.from(
        new Set(runs.map((r) => r.artifact_id).filter((id): id is string => !!id)),
      ),
    [runs],
  )

  const streamIds = useMemo(
    () => [`workflow:${workflowId}`, ...artifactIds.map((id) => `workflow:${id}`)],
    [workflowId, artifactIds],
  )

  const workspace = useActiveWorkspace()
  const queries = useQueries({
    queries: streamIds.map((sid) => ({
      queryKey: ["workflow-events", workspace.id, sid],
      queryFn: () =>
        api.getSpineEvents(workspace.id, { stream_id: sid, limit: 50 }),
      refetchInterval: 5000,
    })),
  })

  const events = useMemo(() => {
    const all: SpineEvent[] = []
    for (const q of queries) {
      if (q.data?.events) all.push(...q.data.events)
    }
    all.sort((a, b) => (a.created_at < b.created_at ? 1 : -1))
    return all.slice(0, 50)
  }, [queries])

  const loading = queries.some((q) => q.isLoading)

  return (
    <Card>
      <CardHeader className="px-4 pb-2 pt-4 md:px-6">
        <CardTitle className="text-base">Events</CardTitle>
      </CardHeader>
      <CardContent className="space-y-2 px-4 pb-4 md:px-6">
        {loading ? (
          <p className="py-4 text-center text-sm text-muted-foreground">Loading…</p>
        ) : events.length === 0 ? (
          <p className="py-4 text-center text-sm text-muted-foreground">
            No events yet. They appear as triggers fire and stages advance.
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
