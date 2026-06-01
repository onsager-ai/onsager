import { useMemo, useState } from "react"
import { Link } from "react-router-dom"
import { useQuery } from "@tanstack/react-query"
import { AlertTriangle, ChevronDown, ChevronRight, Plus, Rocket } from "lucide-react"
import { Button } from "@/components/ui/button"
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card"
import { HitlActionButton } from "@/components/chat/HitlActionButton"
import { SpecPlanDAG } from "@/components/chat/SpecPlanDAG"
import { usePageHeader } from "@/components/layout/PageHeader"
import { mcpCallTool } from "@/lib/mcp-client"
import type { SpecPlan } from "@/lib/spec-plan"
import { useActiveWorkspace } from "@/lib/workspace"

// Spec Plans surface (ADR 0023 / 0025, spec #514). The noun-surface home
// for persisted spec plans: a list, each row with a HitlCard-routed "Run"
// button (`run_spec_plan`). Plans are *authored* in chat (the design
// surface, ADR 0025 F1) but no longer *only* runnable there — this page
// is the launch + monitor entry ADR 0025 requires so chat stops being
// the sole spec-plan-run path.
//
// Listing is MCP-only today (`list_spec_plans`; no REST mirror), so the
// page talks same-origin to the MCP server the same way ChatBuilder
// does. Plan bodies render their authored DAG on demand via the shared
// `SpecPlanDAG` primitive.

interface StoredSpecPlan {
  workspace_id: string
  spec_plan_id: string
  plan: SpecPlan
  created_by: string
  created_at: string
  updated_at: string
}

function parsePlans(text: string | undefined): StoredSpecPlan[] {
  if (!text) return []
  try {
    const parsed = JSON.parse(text) as { spec_plans?: StoredSpecPlan[] }
    return Array.isArray(parsed.spec_plans) ? parsed.spec_plans : []
  } catch {
    return []
  }
}

function planShape(plan: SpecPlan): string {
  const specs = plan.specs?.length ?? 0
  const deps = plan.deps?.length ?? 0
  return `${specs} spec${specs === 1 ? "" : "s"}, ${deps} dep${deps === 1 ? "" : "s"}`
}

export function SpecPlansPage() {
  const workspace = useActiveWorkspace()
  const newPlanPath = `/workspaces/${workspace.slug}/spec-plans/new`
  // Single primary action → inline icon button in the mobile bar
  // (dashboard-ui chrome rule). Memoized so the header effect doesn't
  // re-set on every render.
  const headerActions = useMemo(
    () => (
      <Button
        variant="ghost"
        size="icon"
        aria-label="Create plan"
        title="Create plan"
        render={<Link to={newPlanPath} />}
      >
        <Plus className="h-5 w-5" />
      </Button>
    ),
    [newPlanPath],
  )
  usePageHeader({
    title: "Spec Plans",
    backTo: `/workspaces/${workspace.slug}/workflows`,
    actions: headerActions,
  })

  const plansQuery = useQuery({
    queryKey: ["spec-plans", workspace.id],
    queryFn: async () => {
      const res = await mcpCallTool("list_spec_plans", {
        workspace_id: workspace.id,
      })
      if (res.isError) {
        throw new Error(res.content[0]?.text ?? "Failed to list spec plans")
      }
      return parsePlans(res.content[0]?.text)
    },
  })

  const plans = plansQuery.data ?? []

  return (
    <div className="space-y-6">
      <div className="hidden items-start justify-between gap-3 md:flex">
        <div className="space-y-1">
          <h1 className="text-2xl font-bold tracking-tight">Spec Plans</h1>
          <p className="text-sm text-muted-foreground">
            Author in chat or here, runnable here. Each plan compiles to a
            DAG of specs and runs them in dependency order.
          </p>
        </div>
        <Button className="shrink-0" render={<Link to={newPlanPath} />}>
          <Plus className="h-4 w-4" />
          Create plan
        </Button>
      </div>

      {plansQuery.isLoading ? (
        <LoadingState />
      ) : plansQuery.isError ? (
        <ErrorState message={(plansQuery.error as Error).message} />
      ) : plans.length === 0 ? (
        <EmptyState newPlanPath={newPlanPath} />
      ) : (
        <ul className="space-y-3">
          {plans.map((p) => (
            <li key={p.spec_plan_id}>
              <SpecPlanRow
                plan={p}
                workspaceId={workspace.id}
                onRan={() => plansQuery.refetch()}
              />
            </li>
          ))}
        </ul>
      )}
    </div>
  )
}

function SpecPlanRow({
  plan,
  workspaceId,
  onRan,
}: {
  plan: StoredSpecPlan
  workspaceId: string
  onRan: () => void
}) {
  const [showGraph, setShowGraph] = useState(false)
  const kinds = useMemo(
    () => Array.from(new Set((plan.plan.specs ?? []).map((s) => s.kind))),
    [plan.plan.specs],
  )
  return (
    <Card>
      <CardHeader className="px-4 pb-2 pt-4 md:px-6">
        <div className="flex items-start justify-between gap-3">
          <div className="min-w-0">
            <CardTitle className="truncate font-mono text-base">
              {plan.spec_plan_id}
            </CardTitle>
            <p className="mt-1 text-xs text-muted-foreground">
              {planShape(plan.plan)}
              {kinds.length > 0 ? ` · ${kinds.join(", ")}` : ""}
            </p>
          </div>
          <HitlActionButton
            toolName="run_spec_plan"
            args={{ workspace_id: workspaceId, spec_plan_id: plan.spec_plan_id }}
            onCommitted={onRan}
            variant="default"
            size="sm"
            className="shrink-0"
          >
            <Rocket className="h-4 w-4" />
            Run
          </HitlActionButton>
        </div>
      </CardHeader>
      <CardContent className="px-4 pb-4 md:px-6">
        <Button
          type="button"
          variant="ghost"
          size="sm"
          className="h-7 px-2 text-xs text-muted-foreground"
          onClick={() => setShowGraph((v) => !v)}
          aria-expanded={showGraph}
        >
          {showGraph ? (
            <ChevronDown className="h-3.5 w-3.5" />
          ) : (
            <ChevronRight className="h-3.5 w-3.5" />
          )}
          {showGraph ? "Hide graph" : "View graph"}
        </Button>
        {showGraph ? (
          <div className="mt-2 rounded-md border">
            <SpecPlanDAG plan={plan.plan} />
          </div>
        ) : null}
      </CardContent>
    </Card>
  )
}

function LoadingState() {
  return (
    <div className="space-y-3">
      {[0, 1].map((i) => (
        <div key={i} className="h-24 animate-pulse rounded-lg bg-muted/60" />
      ))}
    </div>
  )
}

function ErrorState({ message }: { message: string }) {
  return (
    <div
      role="alert"
      className="flex items-start gap-2 rounded-md border border-destructive/40 bg-destructive/5 p-3 text-sm text-destructive"
    >
      <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0" />
      <span>Could not load spec plans — {message}</span>
    </div>
  )
}

function EmptyState({ newPlanPath }: { newPlanPath: string }) {
  return (
    <Card>
      <CardContent className="flex flex-col items-center gap-3 py-10 text-center text-sm text-muted-foreground">
        <Rocket className="h-8 w-8 text-muted-foreground/50" />
        <div>No spec plans yet.</div>
        <div className="max-w-sm text-xs text-muted-foreground/80">
          Create a plan here — add specs and their dependencies and submit
          — or describe one in chat. Either way it shows up here, ready to
          launch.
        </div>
        <Button size="sm" className="mt-1" render={<Link to={newPlanPath} />}>
          <Plus className="h-4 w-4" />
          Create plan
        </Button>
      </CardContent>
    </Card>
  )
}
