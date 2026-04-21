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
import {
  Package,
  Activity,
  Shield,
  Server,
  ArrowRight,
  Coins,
  Gauge,
  CheckCircle2,
  Timer,
  AlertTriangle,
  GitBranch,
} from "lucide-react"
import { Link } from "react-router-dom"
import type { SessionSpend, SpineArtifact, SpineEvent } from "@/lib/api"

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
  // Issue #39 — per-session token usage, pulled from the last N
  // session_completed events so the spend card can render without a
  // dedicated accounting endpoint.
  const { data: spendData } = useQuery({
    queryKey: ["session-spend"],
    queryFn: () => api.getSessionSpend(100),
    refetchInterval: 30000,
  })
  // Issue #82 — empty-state CTA banner when no workflow has been set up yet.
  const { data: workflowsData } = useQuery({
    queryKey: ["workflows"],
    queryFn: () => api.listWorkflows(),
    staleTime: 30_000,
  })
  const workflowsCount = workflowsData?.workflows?.length ?? 0

  const artifacts = artifactsData?.artifacts ?? []
  const events = spineData?.events ?? []
  const nodes = nodesData?.nodes ?? []
  const onlineNodes = nodes.filter((n) => n.status === "online").length
  const sessions = spendData ?? []
  const productivity = computeProductivityMetrics(artifacts)

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

      {workflowsCount === 0 && (
        <Card className="border-primary/40 bg-primary/5">
          <CardContent className="flex flex-col items-start gap-3 p-4 md:flex-row md:items-center md:justify-between">
            <div className="flex items-start gap-3">
              <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-full bg-primary/15 text-primary">
                <GitBranch className="h-4 w-4" />
              </div>
              <div>
                <p className="text-sm font-medium">Set up your first workflow</p>
                <p className="text-xs text-muted-foreground">
                  Workflows drive artifacts through stages — without one, the
                  factory is idle.
                </p>
              </div>
            </div>
            <Link
              to="/workflows"
              className="shrink-0 text-sm font-medium text-primary hover:underline"
            >
              Get started →
            </Link>
          </CardContent>
        </Card>
      )}

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

      <ProductivityMetrics metrics={productivity} />

      <SpendCard sessions={sessions} />

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

// ---------------------------------------------------------------------------
// Issue #38 — productivity metrics
// ---------------------------------------------------------------------------

interface ProductivityStats {
  throughputLast24h: number
  yieldRatio: number | null
  avgCycleMs: number | null
  bottleneckState: string | null
  bottleneckCount: number
  releasedCount: number
  totalTracked: number
}

function computeProductivityMetrics(
  artifacts: SpineArtifact[],
): ProductivityStats {
  const now = Date.now()
  const dayAgo = now - 24 * 60 * 60 * 1000

  const released = artifacts.filter((a) => a.state === "released")
  const archivedOrTerminal = artifacts.filter(
    (a) => a.state === "released" || a.state === "archived",
  )
  const tracked = artifacts.length
  const throughputLast24h = released.filter(
    (a) => new Date(a.updated_at).getTime() >= dayAgo,
  ).length

  // Yield: released / (released + archived) among terminal artifacts. A
  // tracker may stay in_progress for long stretches, so skipping non-
  // terminal ones keeps the number interpretable as "of the ones we tried
  // to finish, how many shipped".
  const yieldRatio =
    archivedOrTerminal.length > 0
      ? released.length / archivedOrTerminal.length
      : null

  // Cycle time: average created_at → updated_at among released artifacts.
  // Ignoring archived-without-release cases so a string of aborts doesn't
  // shrink the number to zero.
  let avgCycleMs: number | null = null
  if (released.length > 0) {
    const total = released.reduce((acc, a) => {
      const created = new Date(a.created_at).getTime()
      const updated = new Date(a.updated_at).getTime()
      return acc + Math.max(0, updated - created)
    }, 0)
    avgCycleMs = total / released.length
  }

  // Bottleneck: the non-terminal state with the most artifacts. "If the
  // belt is stuck, where did the boxes pile up?"
  const nonTerminalStates = new Map<string, number>()
  for (const a of artifacts) {
    if (a.state === "released" || a.state === "archived") continue
    nonTerminalStates.set(a.state, (nonTerminalStates.get(a.state) ?? 0) + 1)
  }
  let bottleneckState: string | null = null
  let bottleneckCount = 0
  for (const [state, count] of nonTerminalStates) {
    if (count > bottleneckCount) {
      bottleneckState = state
      bottleneckCount = count
    }
  }

  return {
    throughputLast24h,
    yieldRatio,
    avgCycleMs,
    bottleneckState,
    bottleneckCount,
    releasedCount: released.length,
    totalTracked: tracked,
  }
}

function ProductivityMetrics({ metrics }: { metrics: ProductivityStats }) {
  const yieldDisplay =
    metrics.yieldRatio == null
      ? "—"
      : `${Math.round(metrics.yieldRatio * 100)}%`
  const cycleDisplay = formatDuration(metrics.avgCycleMs)
  const bottleneckDisplay = metrics.bottleneckState
    ? `${metrics.bottleneckState.replace("_", " ")} (${metrics.bottleneckCount})`
    : "—"

  const cards = [
    {
      title: "Throughput (24h)",
      value: metrics.throughputLast24h.toString(),
      icon: Gauge,
      description: `${metrics.releasedCount} released all-time`,
    },
    {
      title: "Yield",
      value: yieldDisplay,
      icon: CheckCircle2,
      description: "released / (released + archived)",
    },
    {
      title: "Avg Cycle Time",
      value: cycleDisplay,
      icon: Timer,
      description: "released artifacts",
    },
    {
      title: "Bottleneck",
      value: bottleneckDisplay,
      icon: AlertTriangle,
      description: "largest non-terminal state",
    },
  ]

  return (
    <div className="grid grid-cols-2 gap-3 md:gap-4 lg:grid-cols-4">
      {cards.map((stat) => (
        <Card key={stat.title}>
          <CardHeader className="flex flex-row items-center justify-between px-3 pb-1 pt-3 md:px-6 md:pb-2 md:pt-6">
            <CardTitle className="text-xs font-medium text-muted-foreground md:text-sm">
              {stat.title}
            </CardTitle>
            <stat.icon className="h-4 w-4 text-muted-foreground" />
          </CardHeader>
          <CardContent className="px-3 pb-3 md:px-6 md:pb-6">
            <div className="text-xl font-bold md:text-2xl">{stat.value}</div>
            <p className="text-[10px] text-muted-foreground md:text-xs">
              {stat.description}
            </p>
          </CardContent>
        </Card>
      ))}
    </div>
  )
}

function formatDuration(ms: number | null): string {
  if (ms == null) return "—"
  if (ms < 60_000) return `${Math.round(ms / 1000)}s`
  if (ms < 3_600_000) return `${Math.round(ms / 60_000)}m`
  if (ms < 86_400_000) return `${(ms / 3_600_000).toFixed(1)}h`
  return `${(ms / 86_400_000).toFixed(1)}d`
}

// ---------------------------------------------------------------------------
// Issue #39 — session spend summary
// ---------------------------------------------------------------------------

function SpendCard({ sessions }: { sessions: SessionSpend[] }) {
  let inputTokens = 0
  let outputTokens = 0
  let cachedTokens = 0
  let reported = 0
  const byModel = new Map<string, number>()
  for (const s of sessions) {
    if (!s.token_usage) continue
    reported += 1
    inputTokens += s.token_usage.input_tokens
    outputTokens += s.token_usage.output_tokens
    cachedTokens +=
      (s.token_usage.cache_read_tokens ?? 0) +
      (s.token_usage.cache_write_tokens ?? 0)
    const model = s.token_usage.model ?? "unknown"
    byModel.set(model, (byModel.get(model) ?? 0) + 1)
  }
  const totalTokens = inputTokens + outputTokens

  return (
    <Card>
      <CardHeader className="flex flex-row items-center justify-between px-4 md:px-6">
        <CardTitle className="text-base md:text-lg">Token Spend</CardTitle>
        <Coins className="h-4 w-4 text-muted-foreground" />
      </CardHeader>
      <CardContent className="px-4 md:px-6">
        {reported === 0 ? (
          <p className="py-4 text-center text-sm text-muted-foreground">
            No session_completed events report token_usage yet. The field is
            optional; Stiglab starts reporting it once the agent runtime
            forwards usage numbers.
          </p>
        ) : (
          <div className="grid grid-cols-2 gap-3 md:grid-cols-4">
            <SpendStat label="Sessions" value={reported.toString()} />
            <SpendStat label="Total tokens" value={formatTokens(totalTokens)} />
            <SpendStat label="Input" value={formatTokens(inputTokens)} />
            <SpendStat label="Output" value={formatTokens(outputTokens)} />
            <SpendStat
              label="Cached"
              value={formatTokens(cachedTokens)}
              description="reads + writes"
            />
            <SpendStat
              label="Models"
              value={Array.from(byModel.keys()).slice(0, 2).join(", ") || "—"}
              description={byModel.size > 2 ? `+${byModel.size - 2} more` : ""}
            />
          </div>
        )}
      </CardContent>
    </Card>
  )
}

function SpendStat({
  label,
  value,
  description,
}: {
  label: string
  value: string
  description?: string
}) {
  return (
    <div>
      <div className="text-xs text-muted-foreground">{label}</div>
      <div className="text-lg font-bold md:text-xl">{value}</div>
      {description && (
        <div className="text-[10px] text-muted-foreground">{description}</div>
      )}
    </div>
  )
}

function formatTokens(n: number): string {
  if (n < 1_000) return n.toString()
  if (n < 1_000_000) return `${(n / 1_000).toFixed(1)}k`
  if (n < 1_000_000_000) return `${(n / 1_000_000).toFixed(2)}M`
  return `${(n / 1_000_000_000).toFixed(2)}B`
}
