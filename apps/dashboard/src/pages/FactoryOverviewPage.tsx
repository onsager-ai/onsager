import { useQuery } from "@tanstack/react-query"
import { api } from "@/lib/api"
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card"
import { Badge } from "@/components/ui/badge"
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table"
import { Package, Activity, Shield, Server, ArrowRight } from "lucide-react"
import { Link } from "react-router-dom"
import type { SpineArtifact, SpineEvent } from "@/lib/api"

const STATE_VARIANT: Record<string, "default" | "secondary" | "destructive" | "outline"> = {
  draft: "outline",
  in_progress: "default",
  under_review: "secondary",
  released: "default",
  archived: "secondary",
}

const STREAM_TYPE_COLORS: Record<string, string> = {
  stiglab: "bg-blue-500/10 text-blue-500 border-blue-500/20",
  synodic: "bg-purple-500/10 text-purple-500 border-purple-500/20",
  forge: "bg-orange-500/10 text-orange-500 border-orange-500/20",
  ising: "bg-green-500/10 text-green-500 border-green-500/20",
}

function PipelineStats({ artifacts }: { artifacts: SpineArtifact[] }) {
  const byState = {
    draft: artifacts.filter((a) => a.state === "draft").length,
    in_progress: artifacts.filter((a) => a.state === "in_progress").length,
    under_review: artifacts.filter((a) => a.state === "under_review").length,
    released: artifacts.filter((a) => a.state === "released").length,
    archived: artifacts.filter((a) => a.state === "archived").length,
  }

  const stages = [
    { label: "Draft", count: byState.draft, color: "text-muted-foreground" },
    { label: "In Progress", count: byState.in_progress, color: "text-blue-500" },
    { label: "Under Review", count: byState.under_review, color: "text-yellow-500" },
    { label: "Released", count: byState.released, color: "text-green-500" },
  ]

  return (
    <Card>
      <CardHeader className="px-4 pb-2 pt-4 md:px-6">
        <CardTitle className="text-base md:text-lg">Artifact Pipeline</CardTitle>
      </CardHeader>
      <CardContent className="px-4 pb-4 md:px-6">
        <div className="flex items-center justify-between gap-2">
          {stages.map((stage, i) => (
            <div key={stage.label} className="flex items-center gap-2">
              <div className="text-center">
                <div className={`text-2xl font-bold md:text-3xl ${stage.color}`}>
                  {stage.count}
                </div>
                <div className="text-xs text-muted-foreground">{stage.label}</div>
              </div>
              {i < stages.length - 1 && (
                <ArrowRight className="h-4 w-4 text-muted-foreground/50" />
              )}
            </div>
          ))}
        </div>
        {byState.archived > 0 && (
          <p className="mt-2 text-xs text-muted-foreground">
            {byState.archived} archived
          </p>
        )}
      </CardContent>
    </Card>
  )
}

export function FactoryOverviewPage() {
  const { data: artifactsData } = useQuery({
    queryKey: ["artifacts"],
    queryFn: api.getArtifacts,
    refetchInterval: 5000,
  })
  const { data: govStats } = useQuery({
    queryKey: ["governance-stats"],
    queryFn: api.getGovernanceStats,
    refetchInterval: 10000,
  })
  const { data: spineData } = useQuery({
    queryKey: ["spine-events-overview"],
    queryFn: () => api.getSpineEvents({ limit: 20 }),
    refetchInterval: 5000,
  })
  const { data: nodesData } = useQuery({
    queryKey: ["nodes"],
    queryFn: api.getNodes,
    refetchInterval: 10000,
  })

  const artifacts = artifactsData?.artifacts ?? []
  const events = spineData?.events ?? []
  const nodes = nodesData?.nodes ?? []
  const onlineNodes = nodes.filter((n) => n.status === "online").length

  const stats = [
    {
      title: "Total Artifacts",
      value: artifacts.length,
      icon: Package,
      description: `${artifacts.filter((a) => a.state !== "archived").length} active`,
    },
    {
      title: "Factory Events",
      value: events.length > 0 ? `${events.length}` : "0",
      icon: Activity,
      description: "Last 20 events",
    },
    {
      title: "Gov. Issues",
      value: govStats?.unresolved ?? 0,
      icon: Shield,
      description: `${govStats?.total ?? 0} total`,
      highlight: (govStats?.unresolved ?? 0) > 0,
    },
    {
      title: "Nodes Online",
      value: `${onlineNodes}/${nodes.length}`,
      icon: Server,
      description: `${nodes.length - onlineNodes} offline`,
    },
  ]

  return (
    <div className="space-y-4 md:space-y-6">
      <div>
        <h1 className="text-xl font-bold tracking-tight md:text-2xl">Factory</h1>
        <p className="text-sm text-muted-foreground">
          Production pipeline overview — artifacts, events, and factory health.
        </p>
      </div>

      <div className="grid grid-cols-2 gap-3 md:gap-4 lg:grid-cols-4">
        {stats.map((stat) => (
          <Card key={stat.title} className={stat.highlight ? "border-yellow-500/50" : ""}>
            <CardHeader className="flex flex-row items-center justify-between px-3 pb-1 pt-3 md:px-6 md:pb-2 md:pt-6">
              <CardTitle className="text-xs font-medium text-muted-foreground md:text-sm">
                {stat.title}
              </CardTitle>
              <stat.icon className={`h-4 w-4 ${stat.highlight ? "text-yellow-500" : "text-muted-foreground"}`} />
            </CardHeader>
            <CardContent className="px-3 pb-3 md:px-6 md:pb-6">
              <div className={`text-xl font-bold md:text-2xl ${stat.highlight ? "text-yellow-500" : ""}`}>
                {stat.value}
              </div>
              <p className="text-[10px] text-muted-foreground md:text-xs">{stat.description}</p>
            </CardContent>
          </Card>
        ))}
      </div>

      <PipelineStats artifacts={artifacts} />

      <div className="grid gap-4 md:grid-cols-2">
        {/* Active Artifacts */}
        <Card>
          <CardHeader className="flex flex-row items-center justify-between px-4 md:px-6">
            <CardTitle className="text-base md:text-lg">Active Artifacts</CardTitle>
            <Link to="/artifacts" className="text-sm text-muted-foreground hover:text-foreground">
              View all
            </Link>
          </CardHeader>
          <CardContent className="px-4 md:px-6">
            {artifacts.filter((a) => a.state !== "archived").length === 0 ? (
              <p className="py-4 text-center text-sm text-muted-foreground">
                No active artifacts. Register one to start the pipeline.
              </p>
            ) : (
              <Table>
                <TableHeader>
                  <TableRow>
                    <TableHead>Name</TableHead>
                    <TableHead>State</TableHead>
                    <TableHead>Version</TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {artifacts
                    .filter((a) => a.state !== "archived")
                    .slice(0, 8)
                    .map((a) => (
                      <TableRow key={a.id}>
                        <TableCell>
                          <Link to={`/artifacts/${a.id}`} className="font-medium hover:underline">
                            {a.name ?? a.id}
                          </Link>
                          <div className="text-xs text-muted-foreground">{a.kind}</div>
                        </TableCell>
                        <TableCell>
                          <Badge variant={STATE_VARIANT[a.state] || "secondary"}>
                            {a.state.replace("_", " ")}
                          </Badge>
                        </TableCell>
                        <TableCell className="font-mono text-sm">v{a.current_version}</TableCell>
                      </TableRow>
                    ))}
                </TableBody>
              </Table>
            )}
          </CardContent>
        </Card>

        {/* Recent Events */}
        <Card>
          <CardHeader className="flex flex-row items-center justify-between px-4 md:px-6">
            <CardTitle className="text-base md:text-lg">Recent Events</CardTitle>
            <Link to="/spine" className="text-sm text-muted-foreground hover:text-foreground">
              View all
            </Link>
          </CardHeader>
          <CardContent className="px-4 md:px-6">
            {events.length === 0 ? (
              <p className="py-4 text-center text-sm text-muted-foreground">
                No events yet. Events appear as the factory processes work.
              </p>
            ) : (
              <div className="space-y-2">
                {events.slice(0, 10).map((e: SpineEvent) => (
                  <div key={e.id} className="flex items-center justify-between gap-2 py-1">
                    <div className="flex items-center gap-2 min-w-0">
                      <Badge
                        variant="outline"
                        className={`shrink-0 text-[10px] ${STREAM_TYPE_COLORS[e.stream_type] || ""}`}
                      >
                        {e.stream_type}
                      </Badge>
                      <span className="truncate text-sm font-mono">{e.event_type}</span>
                    </div>
                    <span className="shrink-0 text-xs text-muted-foreground">
                      {new Date(e.created_at).toLocaleTimeString()}
                    </span>
                  </div>
                ))}
              </div>
            )}
          </CardContent>
        </Card>
      </div>
    </div>
  )
}
