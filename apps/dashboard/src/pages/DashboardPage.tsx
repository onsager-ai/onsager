import { useQuery } from "@tanstack/react-query"
import { Overview } from "@/components/dashboard/Overview"
import { SessionTable } from "@/components/sessions/SessionTable"
import { useNodes } from "@/hooks/useNodes"
import { useSessions } from "@/hooks/useSessions"
import { api } from "@/lib/api"
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card"

export function DashboardPage() {
  const { data: nodesData } = useNodes()
  const { data: sessionsData } = useSessions()
  const { data: govStats } = useQuery({
    queryKey: ["governance-stats"],
    queryFn: api.getGovernanceStats,
    refetchInterval: 10000,
  })
  const { data: artifactsData } = useQuery({
    queryKey: ["artifacts"],
    queryFn: () => api.getArtifacts(),
    refetchInterval: 10000,
  })

  const nodes = nodesData?.nodes ?? []
  const sessions = sessionsData?.sessions ?? []

  return (
    <div className="space-y-4 md:space-y-6">
      <div>
        <h1 className="text-xl font-bold tracking-tight md:text-2xl">Dashboard</h1>
        <p className="text-sm text-muted-foreground">
          Unified overview of the Onsager AI factory.
        </p>
      </div>

      <Overview
        nodes={nodes}
        sessions={sessions}
        governanceStats={govStats}
        artifacts={artifactsData?.artifacts}
      />

      <Card>
        <CardHeader className="px-4 md:px-6">
          <CardTitle className="text-base md:text-lg">Recent Sessions</CardTitle>
        </CardHeader>
        <CardContent className="px-4 md:px-6">
          <SessionTable sessions={sessions.slice(0, 10)} />
        </CardContent>
      </Card>
    </div>
  )
}
