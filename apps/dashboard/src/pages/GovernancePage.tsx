import { useState } from "react"
import { useQuery, useQueryClient } from "@tanstack/react-query"
import { api } from "@/lib/api"
import type { GovernanceEvent, IsingInsightEmittedEvent } from "@/lib/api"
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

const SEVERITY_VARIANT: Record<string, "destructive" | "default" | "secondary" | "outline"> = {
  critical: "destructive",
  high: "destructive",
  medium: "default",
  low: "secondary",
}

const EVENT_TYPES = ["", "tool_call_error", "hallucination", "compliance_violation", "misalignment"]

export function GovernancePage() {
  const [filter, setFilter] = useState("")
  const queryClient = useQueryClient()

  const { data: events, isLoading } = useQuery({
    queryKey: ["governance-events", filter],
    queryFn: () => api.getGovernanceEvents(filter || undefined),
    refetchInterval: 5000,
  })

  const { data: stats } = useQuery({
    queryKey: ["governance-stats"],
    queryFn: api.getGovernanceStats,
    refetchInterval: 5000,
  })

  // Issue #36 — gate-override-rate insights surfaced by Ising. Refreshes
  // less aggressively than governance events since analyzers tick on a 7d
  // window, so per-second polling would burn backend cycles for no signal.
  const { data: insights } = useQuery({
    queryKey: ["ising-insights"],
    queryFn: () => api.getIsingInsights(20),
    refetchInterval: 15000,
  })

  const handleResolve = async (id: string) => {
    const notes = prompt("Resolution notes:")
    if (notes === null) return
    await api.resolveGovernanceEvent(id, notes)
    queryClient.invalidateQueries({ queryKey: ["governance-events"] })
    queryClient.invalidateQueries({ queryKey: ["governance-stats"] })
  }

  return (
    <div className="space-y-4 md:space-y-6">
      <div className="flex flex-col gap-3 md:flex-row md:items-start md:justify-between md:gap-4">
        <div>
          <h1 className="text-xl font-bold tracking-tight md:text-2xl">Governance</h1>
          <p className="text-sm text-muted-foreground">
            AI agent governance events and rules.
          </p>
        </div>
        {stats && (
          <div className="grid grid-cols-3 gap-4 rounded-lg border p-3 md:flex md:gap-6 md:border-0 md:p-0">
            <StatCard label="Total" value={stats.total} />
            <StatCard label="Unresolved" value={stats.unresolved} variant="destructive" />
            <StatCard
              label="Resolution"
              value={`${stats.total > 0 ? Math.round(((stats.total - stats.unresolved) / stats.total) * 100) : 0}%`}
            />
          </div>
        )}
      </div>

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

      <IsingInsightsCard insights={insights ?? []} />

      <Card>
        <CardHeader className="px-4 md:px-6">
          <CardTitle className="text-base md:text-lg">Events</CardTitle>
        </CardHeader>
        <CardContent className="px-4 md:px-6">
          {isLoading ? (
            <p className="py-8 text-center text-muted-foreground">Loading...</p>
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

function EventCard({ event, onResolve }: { event: GovernanceEvent; onResolve: (id: string) => void }) {
  return (
    <div className="flex flex-col gap-2 rounded-lg border p-3">
      <div className="flex items-center gap-2">
        <Badge variant={SEVERITY_VARIANT[event.severity] || "secondary"}>
          {event.severity}
        </Badge>
        <Badge variant="outline">{TYPE_LABELS[event.event_type] || event.event_type}</Badge>
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
        <span className="shrink-0">{new Date(event.created_at).toLocaleString()}</span>
      </div>
      {!event.resolved && (
        <Button variant="outline" size="sm" onClick={() => onResolve(event.id)} className="mt-1">
          Resolve
        </Button>
      )}
    </div>
  )
}

function EventsTable({ events, onResolve }: { events: GovernanceEvent[]; onResolve: (id: string) => void }) {
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
              <Badge variant="outline">{TYPE_LABELS[e.event_type] || e.event_type}</Badge>
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

function IsingInsightsCard({ insights }: { insights: IsingInsightEmittedEvent[] }) {
  return (
    <Card>
      <CardHeader className="px-4 md:px-6">
        <CardTitle className="text-base md:text-lg">Ising Insights</CardTitle>
        <p className="text-xs text-muted-foreground">
          Signals surfaced by the Ising observation loop. Each entry points at
          an artifact kind or subject with notable recent behavior.
        </p>
      </CardHeader>
      <CardContent className="px-4 md:px-6">
        {insights.length === 0 ? (
          <p className="py-6 text-center text-sm text-muted-foreground">
            No insights yet. Ising emits signals as enough factory traffic
            accumulates.
          </p>
        ) : (
          <div className="flex flex-col gap-2">
            {insights.map((i) => (
              <div
                key={i.id}
                className="flex flex-col gap-1 rounded-lg border p-3 md:flex-row md:items-center md:justify-between"
              >
                <div className="flex flex-wrap items-center gap-2">
                  <Badge variant="outline">{i.signal_kind}</Badge>
                  <span className="text-sm font-medium">{i.subject_ref || "—"}</span>
                  <span className="text-xs text-muted-foreground">
                    {i.evidence.length} evidence event{i.evidence.length === 1 ? "" : "s"}
                  </span>
                </div>
                <div className="flex items-center gap-3 text-xs text-muted-foreground">
                  <span>confidence {(i.confidence * 100).toFixed(0)}%</span>
                  <span>{new Date(i.created_at).toLocaleString()}</span>
                </div>
              </div>
            ))}
          </div>
        )}
      </CardContent>
    </Card>
  )
}

function StatCard({ label, value, variant }: { label: string; value: string | number; variant?: string }) {
  return (
    <div className="text-center">
      <div className={`text-lg font-bold md:text-xl ${variant === "destructive" ? "text-destructive" : ""}`}>
        {value}
      </div>
      <div className="text-xs text-muted-foreground">{label}</div>
    </div>
  )
}
