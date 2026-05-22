import { useEffect, useMemo, useRef } from "react"
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
import { ActiveRunsBanner } from "@/components/workflows/ActiveRunsBanner"
import { ArtifactBadge } from "@/components/workflows/ArtifactBadge"
import { ArtifactFlowOverview } from "@/components/workflows/ArtifactFlowOverview"
import { WorkflowActions } from "@/components/workflows/WorkflowActions"
import { WorkflowArtifactsTab } from "@/components/workflows/WorkflowArtifactsTab"
import { WorkflowEventsCard } from "@/components/workflows/WorkflowEventsCard"
import { WorkflowSessionsCard } from "@/components/workflows/WorkflowSessionsCard"
import { WorkflowVerdictsTab } from "@/components/workflows/WorkflowVerdictsTab"
import { WebhookHealthWarning } from "@/components/workflows/WebhookHealthWarning"
import { outputArtifactKind } from "@/components/workflows/workflow-meta"
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
const SEVEN_DAYS_MS = 7 * 24 * 60 * 60 * 1000

function filterRunsToLastSevenDays(runs: WorkflowRun[]): WorkflowRun[] {
  const cutoff = Date.now() - SEVEN_DAYS_MS
  return runs.filter((r) => {
    const t = Date.parse(r.started_at)
    return Number.isNaN(t) || t >= cutoff
  })
}

const STATUS_VARIANT: Record<
  StageRunStatus,
  "default" | "secondary" | "destructive" | "outline"
> = {
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

// Anchored section ids, kept aligned with the legacy `#definition` /
// `#runs` / `#artifacts` / `#verdicts` hash anchors so bookmarks
// survive the collapse from tabs to a single scroll surface (#455).
const SECTION_IDS = ["definition", "runs", "artifacts", "verdicts"] as const
type SectionId = (typeof SECTION_IDS)[number]

// Per-section last_n truncation. Verdicts are denser; show fewer
// inline and link out to the cross-workflow list for the rest (#455).
const LAST_N_RUNS = 10
const LAST_N_VERDICTS = 5

export function WorkflowDetailPage() {
  const { id = "" } = useParams<{ id: string }>()
  const workspace = useActiveWorkspace()
  const isOss = useOSSFlag()

  const { data, isLoading, isError } = useQuery({
    queryKey: ["workflow", id],
    queryFn: () => api.getWorkflow(id),
    enabled: !!id,
  })
  const { data: runsData } = useQuery({
    queryKey: ["workflow-runs", id],
    queryFn: () => api.getWorkflowRuns(id, 50),
    enabled: !!id,
    refetchInterval: 5000,
  })
  const workflow = data?.workflow
  const runs = useMemo(() => {
    const all = runsData?.runs ?? []
    return isOss ? filterRunsToLastSevenDays(all) : all
  }, [runsData, isOss])
  const visibleRuns = useMemo(() => runs.slice(0, LAST_N_RUNS), [runs])
  const runsOverflow = runs.length > LAST_N_RUNS

  // Spec #120 item 3 — webhook delivery health for this workflow's
  // installation.
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

  // Scroll the matched anchored section into view on mount (or when
  // the hash changes from a browser back/forward). One requestAnimation
  // Frame waits for lazy section content to mount before scrolling.
  const scrollRoot = useRef<HTMLDivElement | null>(null)
  useEffect(() => {
    const scrollToHash = () => {
      const raw = window.location.hash.replace(/^#/, "")
      if (!SECTION_IDS.includes(raw as SectionId)) return
      const el = document.getElementById(raw)
      if (el) el.scrollIntoView({ behavior: "smooth", block: "start" })
    }
    const raf = window.requestAnimationFrame(scrollToHash)
    window.addEventListener("hashchange", scrollToHash)
    return () => {
      window.cancelAnimationFrame(raf)
      window.removeEventListener("hashchange", scrollToHash)
    }
  }, [workflow])

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

  const runsListHref = `/workspaces/${workspace.slug}/runs?workflow_id=${encodeURIComponent(workflow.id)}`

  return (
    <div ref={scrollRoot} className="space-y-6 md:space-y-8">
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
      {/* Mobile context strip. */}
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

      {/* Anchored timeline — single scroll surface, four sections.
          `id=` attributes match the legacy `#definition` / `#runs` /
          `#artifacts` / `#verdicts` hash anchors so bookmarks survive
          the collapse from tabs (#455). */}
      <section id="definition" className="scroll-mt-20 space-y-4">
        <SectionHeader title="Definition" />
        <DefinitionSection workflow={workflow} isOss={isOss} />
      </section>

      <section id="runs" className="scroll-mt-20 space-y-4">
        <SectionHeader
          title="Runs"
          actionHref={runsOverflow ? runsListHref : undefined}
          actionLabel="Show all"
        />
        <ActiveRunsBanner
          workflowId={workflow.id}
          workspaceId={workspace.id}
          title="In-flight"
        />
        <RunsList
          runs={visibleRuns}
          stageIds={workflow.stages.map((s) => s.id)}
          workspaceSlug={workspace.slug}
          isOss={isOss}
        />
        <WorkflowEventsCard workflowId={workflow.id} runs={visibleRuns} />
        <WorkflowSessionsCard runs={visibleRuns} />
      </section>

      <section id="artifacts" className="scroll-mt-20 space-y-4">
        <SectionHeader title="Artifacts" />
        <WorkflowArtifactsTab workflowId={workflow.id} />
      </section>

      <section id="verdicts" className="scroll-mt-20 space-y-4">
        <SectionHeader title="Verdicts" />
        <WorkflowVerdictsTab
          workflowId={workflow.id}
          stages={workflow.stages}
          limit={LAST_N_VERDICTS}
        />
      </section>
    </div>
  )
}

function SectionHeader({
  title,
  actionHref,
  actionLabel,
}: {
  title: string
  actionHref?: string
  actionLabel?: string
}) {
  return (
    <div className="flex items-center justify-between gap-2">
      <h2 className="text-lg font-semibold tracking-tight">{title}</h2>
      {actionHref && actionLabel && (
        <Button
          size="sm"
          variant="outline"
          render={<Link to={actionHref} />}
        >
          {actionLabel}
        </Button>
      )}
    </div>
  )
}

function DefinitionSection({
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
