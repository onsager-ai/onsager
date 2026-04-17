import { useState } from "react"
import { useQuery } from "@tanstack/react-query"
import { api, type SpineEvent } from "@/lib/api"
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

const STREAM_TYPE_COLORS: Record<string, string> = {
  stiglab: "bg-blue-500/10 text-blue-500 border-blue-500/20",
  synodic: "bg-purple-500/10 text-purple-500 border-purple-500/20",
  forge: "bg-orange-500/10 text-orange-500 border-orange-500/20",
  ising: "bg-green-500/10 text-green-500 border-green-500/20",
}

const STREAM_TYPES = ["", "stiglab", "synodic", "forge", "ising"]

export function SpinePage() {
  const [streamType, setStreamType] = useState("")

  const { data, isLoading } = useQuery({
    queryKey: ["spine-events", streamType],
    queryFn: () => api.getSpineEvents({
      stream_type: streamType || undefined,
      limit: 100,
    }),
    refetchInterval: 5000,
  })

  const events = data?.events ?? []

  return (
    <div className="space-y-4 md:space-y-6">
      <div>
        <h1 className="text-xl font-bold tracking-tight md:text-2xl">Event Spine</h1>
        <p className="text-sm text-muted-foreground">
          Live view of all factory events across subsystems.
        </p>
      </div>

      <div className="-mx-4 overflow-x-auto px-4 md:mx-0 md:overflow-visible md:px-0">
        <div className="flex gap-2 whitespace-nowrap">
          {STREAM_TYPES.map((t) => (
            <Button
              key={t}
              variant={streamType === t ? "default" : "outline"}
              size="sm"
              onClick={() => setStreamType(t)}
            >
              {t || "All"}
            </Button>
          ))}
        </div>
      </div>

      <Card>
        <CardHeader className="px-4 md:px-6">
          <CardTitle className="text-base md:text-lg">
            Events {events.length > 0 && <span className="text-muted-foreground font-normal">({events.length})</span>}
          </CardTitle>
        </CardHeader>
        <CardContent className="px-4 md:px-6">
          {isLoading ? (
            <p className="py-8 text-center text-muted-foreground">Loading...</p>
          ) : events.length === 0 ? (
            <p className="py-8 text-center text-muted-foreground">
              No spine events yet. Events appear as subsystems process work.
            </p>
          ) : (
            <>
              {/* Mobile: card list */}
              <div className="flex flex-col gap-2 md:hidden">
                {events.map((e) => (
                  <SpineEventCard key={e.id} event={e} />
                ))}
              </div>

              {/* Desktop: table */}
              <div className="hidden md:block">
                <Table>
                  <TableHeader>
                    <TableRow>
                      <TableHead className="w-[60px]">ID</TableHead>
                      <TableHead>Subsystem</TableHead>
                      <TableHead>Event Type</TableHead>
                      <TableHead>Stream ID</TableHead>
                      <TableHead>Actor</TableHead>
                      <TableHead>Time</TableHead>
                    </TableRow>
                  </TableHeader>
                  <TableBody>
                    {events.map((e) => (
                      <TableRow key={e.id}>
                        <TableCell className="font-mono text-xs text-muted-foreground">
                          {e.id}
                        </TableCell>
                        <TableCell>
                          <Badge
                            variant="outline"
                            className={STREAM_TYPE_COLORS[e.stream_type] || ""}
                          >
                            {e.stream_type}
                          </Badge>
                        </TableCell>
                        <TableCell className="font-mono text-sm">{e.event_type}</TableCell>
                        <TableCell className="max-w-[200px] truncate font-mono text-xs text-muted-foreground">
                          {e.stream_id}
                        </TableCell>
                        <TableCell className="text-muted-foreground">{e.actor}</TableCell>
                        <TableCell className="text-xs text-muted-foreground">
                          {new Date(e.created_at).toLocaleString()}
                        </TableCell>
                      </TableRow>
                    ))}
                  </TableBody>
                </Table>
              </div>
            </>
          )}
        </CardContent>
      </Card>
    </div>
  )
}

function SpineEventCard({ event }: { event: SpineEvent }) {
  return (
    <div className="flex flex-col gap-1.5 rounded-lg border p-3">
      <div className="flex items-center gap-2">
        <Badge variant="outline" className={STREAM_TYPE_COLORS[event.stream_type] || ""}>
          {event.stream_type}
        </Badge>
        <span className="truncate font-mono text-sm">{event.event_type}</span>
        <span className="ml-auto shrink-0 font-mono text-xs text-muted-foreground">
          #{event.id}
        </span>
      </div>
      <p className="truncate font-mono text-xs text-muted-foreground">{event.stream_id}</p>
      <div className="flex items-center justify-between text-xs text-muted-foreground">
        <span className="truncate">{event.actor}</span>
        <span className="shrink-0">{new Date(event.created_at).toLocaleString()}</span>
      </div>
    </div>
  )
}
