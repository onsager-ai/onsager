import { useEffect, useMemo, useState } from "react"
import { Link, useParams } from "react-router-dom"
import { useQuery } from "@tanstack/react-query"
import {
  ArrowLeft,
  ArrowRight,
  Circle,
  CircleCheck,
  CircleDot,
  CircleX,
} from "lucide-react"
import { api, type StageRunStatus, type WorkflowRun } from "@/lib/api"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card"
import {
  Tabs,
  TabsContent,
  TabsList,
  TabsTrigger,
} from "@/components/ui/tabs"
import { ActiveRunsBanner } from "@/components/factory/workflows/ActiveRunsBanner"
import { MetaphorBanner } from "@/components/factory/workflows/MetaphorBanner"
import { ArtifactBadge } from "@/components/factory/workflows/ArtifactBadge"
import { ArtifactFlowOverview } from "@/components/factory/workflows/ArtifactFlowOverview"
import { WorkflowActions } from "@/components/factory/workflows/WorkflowActions"
import { WorkflowArtifactsTab } from "@/components/factory/workflows/WorkflowArtifactsTab"
import { WorkflowEventsCard } from "@/components/factory/workflows/WorkflowEventsCard"
import { WorkflowSessionsCard } from "@/components/factory/workflows/WorkflowSessionsCard"
import { WorkflowVerdictsTab } from "@/components/factory/workflows/WorkflowVerdictsTab"
import { WebhookHealthWarning } from "@/components/factory/workflows/WebhookHealthWarning"
import { outputArtifactKind } from "@/components/factory/workflows/workflow-meta"
import { usePageHeader } from "@/components/layout/PageHeader"
import { useOSSFlag } from "@/hooks/useOSSFlag"
import { useActiveWorkspace } from "@/lib/workspace"

// Spec #405: Cloud-vs-OSS surfacing on the workflow detail page is
// inline at the natural limit, not promotional. The `schedule` trigger
// category covers `cron` / `delay` / `interval` kind tags — i.e.
// triggers that need an always-on scheduler. Sourced from the trigger
// registry (`crates/onsager-registry/src/triggers.rs`). Keeping the set
// inline avoids a second registry fetch on this page; if the registry
// gains another `Schedule`-category kind, add it here.
const SCHEDULE_TRIGGER_KIND_TAGS = new Set(["cron", "delay", "interval"])

// Spec #405's run-history view cap on OSS. The locked copy promises
// "Showing last 7 days"; this is the cutoff that keeps the list
// honest. Cloud (full server response) does not apply the filter.
// Hoisted out of the component so `Date.now()` lives outside the
// render body (the `react-hooks/purity` rule rejects impure calls
// in render).
const SEVEN_DAYS_MS = 7 * 24 * 60 * 60 * 1000

function filterRunsToLastSevenDays(runs: WorkflowRun[]): WorkflowRun[] {
  const cutoff = Date.now() - SEVEN_DAYS_MS
  return runs.filter((r) => {
    const t = Date.parse(r.started_at)
    return Number.isNaN(t) || t >= cutoff
  })
}

const STATUS_VARIANT: Record<StageRunStatus, "default" | "secondary" | "destructive" | "outline"> = {
  pending: "outline",
  blocked: "secondary",
  passed: "default",
  failed: "destructive",
}

const STATUS_ICON: Record<StageRunStatus, typeof Circle> = {
  pending: Circle,
  blocked: CircleDot,
  passed: CircleCheck,
  failed: CircleX,
}

const TAB_VALUES = ["definition", "runs", "artifacts", "verdicts"] as const
type TabValue = (typeof TAB_VALUES)[number]
const DEFAULT_TAB: TabValue = "runs"

function readTabFromHash(): TabValue {
  if (typeof window === "undefined") return DEFAULT_TAB
  const raw = window.location.hash.replace(/^#/, "")
  return (TAB_VALUES as readonly string[]).includes(raw)
    ? (raw as TabValue)
    : DEFAULT_TAB
}

export function WorkflowDetailPage() {
  const { id = "" } = useParams<{ id: string }>()
  const workspace = useActiveWorkspace()
  const isOss = useOSSFlag()

  const { data, isLoading, isError } = useQuery({
    queryKey: ["workflow", id],
    queryFn: () => api.getWorkflow(id),
    enabled: !!id,
  })
  // Live view of artifacts flowing through stages. The spine bus emits
  // `forge.stage_*` events. Until a push channel (WebSocket/SSE) lands,
  // poll at 5s — matches the rest of the dashboard's fast-refresh cadence
  // without waking the mobile radio every 2s.
  const { data: runsData } = useQuery({
    queryKey: ["workflow-runs", id],
    queryFn: () => api.getWorkflowRuns(id, 50),
    enabled: !!id,
    refetchInterval: 5000,
  })
  const workflow = data?.workflow
  // Spec #405: keep the OSS "Showing last 7 days" copy truthful.
  // Backend retention enforcement is the follow-up "Cloud retention
  // job" spec; the dashboard cap is a view-side filter so the line
  // and the list agree. Cloud renders the full server response.
  const runs = useMemo(() => {
    const all = runsData?.runs ?? []
    return isOss ? filterRunsToLastSevenDays(all) : all
  }, [runsData, isOss])

  // Spec #120 item 3 — webhook delivery health for this workflow's
  // installation. The workspace-scoped endpoint returns every
  // installation in one call; we pick our row by install_id.
  const { data: healthData } = useQuery({
    queryKey: ["webhook-deliveries-health", workspace.id],
    queryFn: () => api.getWorkspaceWebhookDeliveriesHealth(workspace.id),
    enabled: !!workflow,
    staleTime: 60_000,
  })
  const installHealth = useMemo(() => {
    if (!workflow) return undefined
    return healthData?.installations.find(
      (h) => String(h.install_id) === workflow.trigger.install_id,
    )
  }, [healthData, workflow])

  const [tab, setTab] = useState<TabValue>(() => readTabFromHash())

  // Keep state and the URL hash in sync. Initial state is seeded from the
  // hash so a `#artifacts` deep link lands on the right tab; subsequent
  // navigations (browser back/forward, manual edits) update via the
  // `hashchange` listener; tab clicks push via `replaceState` so the
  // back stack doesn't fill with every glance at the page.
  useEffect(() => {
    const onHashChange = () => setTab(readTabFromHash())
    window.addEventListener("hashchange", onHashChange)
    return () => window.removeEventListener("hashchange", onHashChange)
  }, [])

  const handleTabChange = (value: TabValue) => {
    setTab(value)
    const next = `#${value}`
    if (window.location.hash !== next) {
      window.history.replaceState(null, "", next)
    }
  }

  // Mobile chrome: back arrow + workflow name + ⋯ overflow with
  // Pause/Delete live in the global top bar. Desktop renders the
  // page-level block below (md:flex). Header stays registered while
  // loading so the bar doesn't flicker between "Onsager" and the
  // workflow name. Memoize the JSX action node — the dashboard-ui rule
  // says JSX `actions` should be `useMemo`d.
  const headerActions = useMemo(
    () =>
      workflow ? <WorkflowActions workflow={workflow} variant="menu" /> : null,
    [workflow],
  )
  usePageHeader({
    title: workflow?.name ?? "Workflow",
    backTo: `/workspaces/${workspace.slug}/workflows`,
    actions: headerActions,
  })

  if (isLoading) return <p className="text-sm text-muted-foreground">Loading…</p>
  if (isError || !workflow) {
    return (
      <div className="space-y-3">
        <p className="text-sm text-destructive">Couldn&apos;t load workflow.</p>
      </div>
    )
  }

  return (
    <div className="space-y-4 md:space-y-6">
      <MetaphorBanner />
      {/* Desktop-only page header. Mobile uses the global top bar. */}
      <div className="hidden space-y-2 md:block">
        <BackLink workspaceSlug={workspace.slug} />
        <div className="flex items-center justify-between gap-2">
          <div className="min-w-0">
            <h1 className="truncate text-2xl font-bold tracking-tight">
              {workflow.name}
            </h1>
            <p className="truncate text-sm text-muted-foreground">
              {workflow.trigger.repo_owner}/{workflow.trigger.repo_name}
              {workflow.trigger.label ? ` · ${workflow.trigger.label}` : ""}
            </p>
          </div>
          <Badge variant={workflow.status === "active" ? "default" : "outline"}>
            {workflow.status}
          </Badge>
        </div>
        <WorkflowActions workflow={workflow} />
      </div>
      {/* Mobile context strip: repo + status badge sit just under the
          global header so users still see them on small screens. */}
      <div className="flex items-center justify-between gap-2 md:hidden">
        <p className="min-w-0 truncate text-sm text-muted-foreground">
          {workflow.trigger.repo_owner}/{workflow.trigger.repo_name}
          {workflow.trigger.label ? ` · ${workflow.trigger.label}` : ""}
        </p>
        <Badge variant={workflow.status === "active" ? "default" : "outline"}>
          {workflow.status}
        </Badge>
      </div>

      <WebhookHealthWarning health={installHealth} variant="block" />


      <Tabs value={tab} onValueChange={(v) => handleTabChange(v as TabValue)}>
        <TabsList className="w-full justify-start overflow-x-auto md:w-auto">
          <TabsTrigger value="definition">Definition</TabsTrigger>
          <TabsTrigger value="runs">Runs</TabsTrigger>
          <TabsTrigger value="artifacts">Artifacts</TabsTrigger>
          <TabsTrigger value="verdicts">Verdicts</TabsTrigger>
        </TabsList>

        <TabsContent value="definition" className="space-y-4 pt-4 md:space-y-6">
          <DefinitionTab workflow={workflow} isOss={isOss} />
        </TabsContent>

        <TabsContent value="runs" className="space-y-4 pt-4 md:space-y-6">
          <ActiveRunsBanner
            workflowId={workflow.id}
            workspaceId={workspace.id}
            title="In-flight"
          />
          <RunsList
            runs={runs}
            stageIds={workflow.stages.map((s) => s.id)}
            workspaceSlug={workspace.slug}
            isOss={isOss}
          />
          <WorkflowEventsCard workflowId={workflow.id} runs={runs} />
          <WorkflowSessionsCard runs={runs} />
        </TabsContent>

        <TabsContent value="artifacts" className="space-y-4 pt-4 md:space-y-6">
          <WorkflowArtifactsTab workflowId={workflow.id} />
        </TabsContent>

        <TabsContent value="verdicts" className="space-y-4 pt-4 md:space-y-6">
          <WorkflowVerdictsTab
            workflowId={workflow.id}
            stages={workflow.stages}
          />
        </TabsContent>
      </Tabs>
    </div>
  )
}

function DefinitionTab({
  workflow,
  isOss,
}: {
  workflow: NonNullable<Awaited<ReturnType<typeof api.getWorkflow>>["workflow"]>
  isOss: boolean
}) {
  const isSchedule = SCHEDULE_TRIGGER_KIND_TAGS.has(workflow.trigger.kind_tag)
  return (
    <Card>
      <CardHeader className="px-4 pb-2 pt-4 md:px-6">
        <CardTitle className="text-base">Stages</CardTitle>
      </CardHeader>
      <CardContent className="space-y-3 px-4 pb-4 md:px-6">
        <div className="rounded-md border bg-muted/30 px-3 py-2">
          <div className="mb-1 text-[10px] uppercase tracking-wide text-muted-foreground">
            Flow
          </div>
          <ArtifactFlowOverview
            triggerLabel={workflow.trigger.label ?? ""}
            stages={workflow.stages}
          />
          {/* Spec #405: OSS users running schedule triggers (cron /
              delay / interval) discover the always-on-scheduler limit
              here, where it bites. The line is inline, not a modal. */}
          {isOss && isSchedule && (
            <p className="mt-2 text-xs text-muted-foreground">
              Runs while this Onsager process is running. For 24/7
              schedules,{" "}
              <a
                href="https://app.onsager.ai"
                target="_blank"
                rel="noopener noreferrer"
                className="underline underline-offset-2 hover:text-foreground"
              >
                use Cloud →
              </a>
            </p>
          )}
        </div>
        {workflow.stages.map((s, i) => {
          const output = outputArtifactKind(s.gate_kind, s.artifact_kind)
          const transforms = output !== s.artifact_kind
          return (
            <div
              key={s.id}
              className="flex items-center justify-between gap-2 rounded-md border px-3 py-2"
            >
              <div className="min-w-0 space-y-1">
                <div className="truncate text-sm font-medium">
                  {i + 1}. {s.name}
                </div>
                <div className="flex flex-wrap items-center gap-1.5">
                  <span className="text-[10px] uppercase tracking-wide text-muted-foreground">
                    {s.gate_kind}
                  </span>
                  <span className="text-muted-foreground/50">·</span>
                  <span className="text-[10px] uppercase tracking-wide text-muted-foreground">
                    in
                  </span>
                  <ArtifactBadge kind={s.artifact_kind} />
                  {transforms && (
                    <>
                      <ArrowRight className="h-3 w-3 text-muted-foreground" />
                      <span className="text-[10px] uppercase tracking-wide text-muted-foreground">
                        out
                      </span>
                      <ArtifactBadge kind={output} />
                    </>
                  )}
                </div>
              </div>
            </div>
          )
        })}
      </CardContent>
    </Card>
  )
}

function RunsList({
  runs,
  stageIds,
  workspaceSlug,
  isOss,
}: {
  runs: WorkflowRun[]
  stageIds: string[]
  workspaceSlug: string
  isOss: boolean
}) {
  return (
    <Card>
      <CardHeader className="px-4 pb-2 pt-4 md:px-6">
        <CardTitle className="text-base">Run history</CardTitle>
        {/* Spec #405: OSS dashboards cap the run-history view at 7
            days; the line surfaces the Cloud value (90-day retention)
            at the moment the user hits the wall. Informative, not
            promotional. The actual retention enforcement is a follow-
            up spec; this is the surfacing half. */}
        {isOss && (
          <p className="text-xs text-muted-foreground">
            Showing last 7 days · Cloud retains 90.
          </p>
        )}
      </CardHeader>
      <CardContent className="space-y-3 px-4 pb-4 md:px-6">
        {runs.length === 0 ? (
          <p className="py-4 text-center text-sm text-muted-foreground">
            No runs yet. Tag an issue with the trigger label to kick one off.
          </p>
        ) : (
          runs.map((r) => (
            <RunRow
              key={r.id}
              run={r}
              stageIds={stageIds}
              workspaceSlug={workspaceSlug}
            />
          ))
        )}
      </CardContent>
    </Card>
  )
}

function RunRow({
  run,
  stageIds,
  workspaceSlug,
}: {
  run: WorkflowRun
  stageIds: string[]
  workspaceSlug: string
}) {
  const byStage = new Map(run.stages.map((s) => [s.stage_id, s.status]))
  return (
    <Link
      to={`/workspaces/${workspaceSlug}/runs/${run.id}`}
      className="block space-y-2 rounded-md border p-3 hover:bg-muted/50"
    >
      <div className="flex items-center justify-between gap-2">
        <div className="min-w-0 text-sm font-mono truncate">
          {run.artifact_id ?? run.id}
        </div>
        <Badge variant={STATUS_VARIANT[run.status]}>{run.status}</Badge>
      </div>
      <div className="flex items-center gap-1">
        {stageIds.map((sid) => {
          const status = byStage.get(sid) ?? "pending"
          const Icon = STATUS_ICON[status]
          return (
            <Icon
              key={sid}
              aria-label={status}
              className={`h-4 w-4 ${iconClass(status)}`}
            />
          )
        })}
      </div>
    </Link>
  )
}

function iconClass(status: StageRunStatus): string {
  switch (status) {
    case "passed":
      return "text-green-500"
    case "failed":
      return "text-destructive"
    case "blocked":
      return "text-yellow-500"
    default:
      return "text-muted-foreground"
  }
}

function BackLink({ workspaceSlug }: { workspaceSlug: string }) {
  return (
    <Button
      variant="ghost"
      size="sm"
      render={<Link to={`/workspaces/${workspaceSlug}/workflows`} />}
    >
      <ArrowLeft className="h-4 w-4" />
      Workflows
    </Button>
  )
}
