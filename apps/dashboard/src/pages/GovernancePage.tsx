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

  const handleResolve = async (id: string) => {
    const notes = prompt("Resolution notes:")
    if (notes === null) return
    await api.resolveGovernanceEvent(id, notes)
    queryClient.invalidateQueries({ queryKey: ["governance-events"] })
    queryClient.invalidateQueries({ queryKey: ["governance-stats"] })
  }

  return (
    <div className="space-y-4 md:space-y-6">
      <div className="flex items-start justify-between gap-4">
        <div>
          <h1 className="text-xl font-bold tracking-tight md:text-2xl">Governance</h1>
          <p className="text-sm text-muted-foreground">
            AI agent governance events and rules.
          </p>
        </div>
        {stats && (
          <div className="flex gap-6 text-sm">
            <StatCard label="Total" value={stats.total} />
            <StatCard label="Unresolved" value={stats.unresolved} variant="destructive" />
            <StatCard
              label="Resolution"
              value={`${stats.total > 0 ? Math.round(((stats.total - stats.unresolved) / stats.total) * 100) : 0}%`}
            />
          </div>
        )}
      </div>

      <div className="flex gap-2">
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
            <EventsTable events={events} onResolve={handleResolve} />
          )}
        </CardContent>
      </Card>
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

function StatCard({ label, value, variant }: { label: string; value: string | number; variant?: string }) {
  return (
    <div className="text-center">
      <div className={`text-xl font-bold ${variant === "destructive" ? "text-destructive" : ""}`}>
        {value}
      </div>
      <div className="text-xs text-muted-foreground">{label}</div>
    </div>
  )
}
