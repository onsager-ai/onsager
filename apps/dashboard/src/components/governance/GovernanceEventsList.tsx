import { useState } from "react"
import { useQuery, useQueryClient } from "@tanstack/react-query"
import { api } from "@/lib/api"
import type { GovernanceEvent } from "@/lib/api"
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card"
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

const TYPE_LABELS: Record<string, string> = {
  tool_call_error: "Tool Error",
  hallucination: "Hallucination",
  compliance_violation: "Compliance",
  misalignment: "Misalignment",
}

const SEVERITY_VARIANT: Record<
  string,
  "destructive" | "default" | "secondary" | "outline"
> = {
  critical: "destructive",
  high: "destructive",
  medium: "default",
  low: "secondary",
}

const EVENT_TYPES = [
  "",
  "tool_call_error",
  "hallucination",
  "compliance_violation",
  "misalignment",
]

interface GovernanceEventsListProps {
  workspaceId: string
}

export function GovernanceEventsList({ workspaceId }: GovernanceEventsListProps) {
  const queryClient = useQueryClient()
  const [filter, setFilter] = useState("")

  const { data: events, isLoading, isError, error } = useQuery({
    queryKey: ["governance-events", workspaceId, filter],
    queryFn: () => api.getGovernanceEvents(workspaceId, filter || undefined),
    refetchInterval: 5000,
  })

  const handleResolve = async (id: string) => {
    const notes = prompt("Resolution notes:")
    if (notes === null) return
    await api.resolveGovernanceEvent(id, notes)
    queryClient.invalidateQueries({ queryKey: ["governance-events"] })
    queryClient.invalidateQueries({ queryKey: ["governance-stats"] })
  }

  return (
    <div className="space-y-4">
      <div className="-mx-4 overflow-x-auto px-4 md:mx-0 md:overflow-visible md:px-0">
        <div className="flex gap-2 whitespace-nowrap">
          {EVENT_TYPES.map((t) => (
            <Button
              key={t}
              variant={filter === t ? "default" : "outline"}
              size="sm"
              onClick={() => setFilter(t)}
            >
              {t ? TYPE_LABELS[t] || t : "All"}
            </Button>
          ))}
        </div>
      </div>

      <Card>
        <CardHeader className="px-4 md:px-6">
          <CardTitle className="text-base md:text-lg">Events</CardTitle>
        </CardHeader>
        <CardContent className="px-4 md:px-6">
          {isLoading ? (
            <p className="py-8 text-center text-muted-foreground">Loading...</p>
          ) : isError ? (
            <p className="py-8 text-center text-sm text-destructive">
              Failed to load events
              {error instanceof Error ? `: ${error.message}` : "."}
            </p>
          ) : !events || events.length === 0 ? (
            <p className="py-8 text-center text-muted-foreground">
              No governance events. Submit events via the synodic CLI or API.
            </p>
          ) : (
            <>
              {/* Mobile: card list */}
              <div className="flex flex-col gap-2 md:hidden">
                {events.map((e) => (
                  <EventCard key={e.id} event={e} onResolve={handleResolve} />
                ))}
              </div>

              {/* Desktop: table */}
              <div className="hidden md:block">
                <EventsTable events={events} onResolve={handleResolve} />
              </div>
            </>
          )}
        </CardContent>
      </Card>
    </div>
  )
}

function EventCard({
  event,
  onResolve,
}: {
  event: GovernanceEvent
  onResolve: (id: string) => void
}) {
  return (
    <div className="flex flex-col gap-2 rounded-lg border p-3">
      <div className="flex items-center gap-2">
        <Badge variant={SEVERITY_VARIANT[event.severity] || "secondary"}>
          {event.severity}
        </Badge>
        <Badge variant="outline">
          {TYPE_LABELS[event.event_type] || event.event_type}
        </Badge>
        <div className="ml-auto">
          {event.resolved ? (
            <Badge variant="secondary">Resolved</Badge>
          ) : (
            <Badge variant="destructive">Open</Badge>
          )}
        </div>
      </div>
      <p className="text-sm font-medium">{event.title}</p>
      <div className="flex items-center justify-between text-xs text-muted-foreground">
        <span className="truncate">{event.source}</span>
        <span className="shrink-0">
          {new Date(event.created_at).toLocaleString()}
        </span>
      </div>
      {!event.resolved && (
        <Button
          variant="outline"
          size="sm"
          onClick={() => onResolve(event.id)}
          className="mt-1"
        >
          Resolve
        </Button>
      )}
    </div>
  )
}

function EventsTable({
  events,
  onResolve,
}: {
  events: GovernanceEvent[]
  onResolve: (id: string) => void
}) {
  return (
    <Table>
      <TableHeader>
        <TableRow>
          <TableHead>Type</TableHead>
          <TableHead>Severity</TableHead>
          <TableHead>Title</TableHead>
          <TableHead>Source</TableHead>
          <TableHead>Status</TableHead>
          <TableHead>Created</TableHead>
          <TableHead />
        </TableRow>
      </TableHeader>
      <TableBody>
        {events.map((e) => (
          <TableRow key={e.id}>
            <TableCell>
              <Badge variant="outline">
                {TYPE_LABELS[e.event_type] || e.event_type}
              </Badge>
            </TableCell>
            <TableCell>
              <Badge variant={SEVERITY_VARIANT[e.severity] || "secondary"}>
                {e.severity}
              </Badge>
            </TableCell>
            <TableCell className="max-w-[300px] truncate">{e.title}</TableCell>
            <TableCell className="text-muted-foreground">{e.source}</TableCell>
            <TableCell>
              {e.resolved ? (
                <Badge variant="secondary">Resolved</Badge>
              ) : (
                <Badge variant="destructive">Open</Badge>
              )}
            </TableCell>
            <TableCell className="text-xs text-muted-foreground">
              {new Date(e.created_at).toLocaleString()}
            </TableCell>
            <TableCell>
              {!e.resolved && (
                <Button variant="ghost" size="sm" onClick={() => onResolve(e.id)}>
                  Resolve
                </Button>
              )}
            </TableCell>
          </TableRow>
        ))}
      </TableBody>
    </Table>
  )
}
