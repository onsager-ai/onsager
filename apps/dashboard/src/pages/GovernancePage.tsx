import { useQuery, useQueryClient } from "@tanstack/react-query"
import { api } from "@/lib/api"
import { useActiveWorkspace } from "@/lib/workspace"
import type {
  IsingInsightEmittedEvent,
  RuleProposal,
} from "@/lib/api"
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { usePageHeader } from "@/components/layout/PageHeader"
import { GovernanceEventsList } from "@/components/governance/GovernanceEventsList"

export function GovernancePage() {
  usePageHeader({ title: "Governance" })
  const workspace = useActiveWorkspace()
  const queryClient = useQueryClient()

  const { data: stats } = useQuery({
    queryKey: ["governance-stats", workspace.id],
    queryFn: () => api.getGovernanceStats(workspace.id),
    refetchInterval: 5000,
  })

  // Issue #36 — gate-override-rate insights surfaced by Ising. Refreshes
  // less aggressively than governance events since analyzers tick on a 7d
  // window, so per-second polling would burn backend cycles for no signal.
  const { data: insights } = useQuery({
    queryKey: ["ising-insights", workspace.id],
    queryFn: () => api.getIsingInsights(workspace.id, 20),
    refetchInterval: 15000,
  })

  // Issue #36 Step 2 — pending rule proposals. Same slow-refresh cadence as
  // insights; proposals churn at most once per ising tick.
  const { data: pendingProposals } = useQuery({
    queryKey: ["rule-proposals-pending", workspace.id],
    queryFn: () => api.getRuleProposals(workspace.id, "pending"),
    refetchInterval: 15000,
  })

  const handleProposalResolve = async (
    id: string,
    status: "approved" | "rejected",
  ) => {
    const notes = prompt(`Notes for ${status} proposal (optional):`) ?? undefined
    await api.resolveRuleProposal(id, status, notes || undefined)
    queryClient.invalidateQueries({ queryKey: ["rule-proposals-pending"] })
  }

  return (
    <div className="space-y-4 md:space-y-6">
      <div className="flex flex-col gap-3 md:flex-row md:items-start md:justify-between md:gap-4">
        <div>
          <h1 className="hidden text-2xl font-bold tracking-tight md:block">Governance</h1>
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

      <RuleProposalsCard
        proposals={pendingProposals ?? []}
        onResolve={handleProposalResolve}
      />

      <IsingInsightsCard insights={insights ?? []} />

      <GovernanceEventsList workspaceId={workspace.id} />
    </div>
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

function RuleProposalsCard({
  proposals,
  onResolve,
}: {
  proposals: RuleProposal[]
  onResolve: (id: string, status: "approved" | "rejected") => void
}) {
  return (
    <Card>
      <CardHeader className="px-4 md:px-6">
        <CardTitle className="text-base md:text-lg">Rule Proposals</CardTitle>
        <p className="text-xs text-muted-foreground">
          Rule changes Ising queued for review. Approve to apply; reject to
          keep the rule as-is. Safe-auto proposals already applied and land
          here in the "approved" view only.
        </p>
      </CardHeader>
      <CardContent className="px-4 md:px-6">
        {proposals.length === 0 ? (
          <p className="py-6 text-center text-sm text-muted-foreground">
            No pending proposals. Ising surfaces them when a signal crosses
            the rule-proposal threshold.
          </p>
        ) : (
          <div className="flex flex-col gap-2">
            {proposals.map((p) => {
              const action = p.proposed_action as {
                action?: string
                rule_id?: string
                subject_ref?: string
              }
              const actionLabel =
                action.action === "retire"
                  ? `retire rule ${action.rule_id}`
                  : action.action === "rewrite"
                    ? `rewrite rule ${action.rule_id}`
                    : action.action === "introduce"
                      ? `introduce rule for ${action.subject_ref}`
                      : action.action ?? "change"
              return (
                <div
                  key={p.id}
                  className="flex flex-col gap-2 rounded-lg border p-3"
                >
                  <div className="flex flex-wrap items-center gap-2">
                    <Badge variant="outline">{p.signal_kind}</Badge>
                    <Badge
                      variant={
                        p.class === "safe_auto" ? "secondary" : "default"
                      }
                    >
                      {p.class.replace("_", " ")}
                    </Badge>
                    <span className="text-sm font-medium">{actionLabel}</span>
                    <span className="ml-auto text-xs text-muted-foreground">
                      {(p.confidence * 100).toFixed(0)}% confidence
                    </span>
                  </div>
                  <p className="text-sm text-muted-foreground">{p.rationale}</p>
                  <div className="flex items-center justify-between text-xs text-muted-foreground">
                    <span className="truncate">
                      subject: {p.subject_ref}
                    </span>
                    <span>{new Date(p.created_at).toLocaleString()}</span>
                  </div>
                  <div className="flex gap-2">
                    <Button
                      variant="default"
                      size="sm"
                      onClick={() => onResolve(p.id, "approved")}
                    >
                      Approve
                    </Button>
                    <Button
                      variant="outline"
                      size="sm"
                      onClick={() => onResolve(p.id, "rejected")}
                    >
                      Reject
                    </Button>
                  </div>
                </div>
              )
            })}
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
