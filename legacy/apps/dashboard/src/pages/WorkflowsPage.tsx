import { useMemo, useState } from "react"
import { Link, useSearchParams } from "react-router-dom"
import { useQuery } from "@tanstack/react-query"
import { GitBranch, Plus } from "lucide-react"
import { api, type ReadinessFilter, type AutofireFilter } from "@/lib/api"
import { useActiveWorkspace } from "@/lib/workspace"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card"
import { ActiveRunsBanner } from "@/components/workflows/ActiveRunsBanner"
import { LocalDraftsCard } from "@/components/workflows/LocalDraftsCard"
import { WebhookHealthWarning } from "@/components/workflows/WebhookHealthWarning"
import { WorkflowBuilderSheet } from "@/components/workflows/WorkflowBuilderSheet"
import { WorkflowStatusChips } from "@/components/workflows/WorkflowStatusChips"
import { usePageHeader } from "@/components/layout/PageHeader"

const READINESS_VALUES: ReadinessFilter[] = ["all", "drafted", "ready"]
const AUTOFIRE_VALUES: AutofireFilter[] = ["all", "off", "active", "paused"]

function readReadiness(raw: string | null): ReadinessFilter {
  if (!raw) return "all"
  return (READINESS_VALUES as readonly string[]).includes(raw)
    ? (raw as ReadinessFilter)
    : "all"
}

function readAutofire(raw: string | null): AutofireFilter {
  if (!raw) return "all"
  return (AUTOFIRE_VALUES as readonly string[]).includes(raw)
    ? (raw as AutofireFilter)
    : "all"
}

export function WorkflowsPage() {
  usePageHeader({ title: "Workflows" })
  const workspace = useActiveWorkspace()
  const [creating, setCreating] = useState(false)
  const [params, setParams] = useSearchParams()

  const readinessFilter = readReadiness(params.get("readiness"))
  const autofireFilter = readAutofire(params.get("autofire"))

  const { data, isLoading } = useQuery({
    queryKey: ["workflows", workspace.id, readinessFilter, autofireFilter],
    queryFn: () =>
      api.listWorkflows(workspace.id, {
        readiness: readinessFilter,
        autofire: autofireFilter,
      }),
  })
  const workflows = useMemo(() => data?.workflows ?? [], [data])
  const counts = data?.counts ?? {
    all: 0,
    readiness: { drafted: 0, ready: 0 },
    autofire: { off: 0, active: 0, paused: 0 },
  }
  const filtered = readinessFilter !== "all" || autofireFilter !== "all"

  // Spec #120 item 3 — warn inline on the card when the installation's
  // recent GitHub deliveries include non-2xx responses. One workspace-
  // scoped fetch covers every workflow's installation; the card looks up
  // its row by install_id. Skipped when there are no workflows yet (no
  // installations to grade either).
  const { data: healthData } = useQuery({
    queryKey: ["webhook-deliveries-health", workspace.id],
    queryFn: () => api.getWorkspaceWebhookDeliveriesHealth(workspace.id),
    enabled: workflows.length > 0,
    staleTime: 60_000,
  })
  const healthByInstall = new Map(
    (healthData?.installations ?? []).map((h) => [String(h.install_id), h]),
  )

  const setReadiness = (next: ReadinessFilter) => {
    const updated = new URLSearchParams(params)
    if (next === "all") updated.delete("readiness")
    else updated.set("readiness", next)
    setParams(updated, { replace: true })
  }

  const setAutofire = (next: AutofireFilter) => {
    const updated = new URLSearchParams(params)
    if (next === "all") updated.delete("autofire")
    else updated.set("autofire", next)
    setParams(updated, { replace: true })
  }

  return (
    <div className="space-y-4 md:space-y-6">
      <div className="flex items-center justify-between gap-2">
        <div>
          <h1 className="hidden text-2xl font-bold tracking-tight md:block">Workflows</h1>
          <p className="text-sm text-muted-foreground">
            Triggers that drive artifacts through stages. Tap a workflow to view.
          </p>
        </div>
        <Button onClick={() => setCreating(true)} size="sm" className="shrink-0">
          <Plus className="h-4 w-4" />
          Create workflow
        </Button>
      </div>

      <WorkflowStatusChips
        readiness={readinessFilter}
        autofire={autofireFilter}
        counts={counts}
        onReadinessChange={setReadiness}
        onAutofireChange={setAutofire}
      />

      {/* Drafted view (#467 / ADR 0019). Client-side drafts live in
          localStorage (spec #401); the view absorbs DraftStrip's
          quick-jump / continue-in-chat / delete affordances. Spine-side
          drafts continue to render in the list below. */}
      {readinessFilter === "drafted" && <LocalDraftsCard />}

      <ActiveRunsBanner workspaceId={workspace.id} />

      {isLoading ? (
        <p className="text-sm text-muted-foreground">Loading…</p>
      ) : workflows.length === 0 ? (
        <Card>
          <CardContent className="flex flex-col items-center gap-3 py-10 text-center">
            <div className="flex h-12 w-12 items-center justify-center rounded-full bg-primary/10 text-primary">
              <GitBranch className="h-6 w-6" />
            </div>
            <div>
              <p className="text-base font-medium">
                {filtered ? "No matching workflows" : "No workflows yet"}
              </p>
              <p className="text-sm text-muted-foreground">
                {filtered
                  ? "Adjust the filters above or create a workflow via ⌘K."
                  : "Set one up and Onsager starts responding to GitHub events. Open ⌘K to create one."}
              </p>
            </div>
            {!filtered && (
              <Button onClick={() => setCreating(true)} size="lg">
                <Plus className="h-4 w-4" />
                Create your first workflow
              </Button>
            )}
          </CardContent>
        </Card>
      ) : (
        <div className="space-y-3">
          {workflows.map((w) => (
            <Link
              key={w.id}
              to={`/workspaces/${workspace.slug}/workflows/${w.id}`}
              className="block focus:outline-none focus-visible:ring-2 focus-visible:ring-ring rounded-md"
            >
              <Card className="transition hover:border-primary/40">
                <CardHeader className="flex flex-row items-center justify-between gap-2 px-4 pb-2 pt-4">
                  <CardTitle className="truncate text-base">{w.name}</CardTitle>
                  <Badge variant={w.status === "active" ? "default" : "outline"}>
                    {w.status}
                  </Badge>
                </CardHeader>
                <CardContent className="space-y-1 px-4 pb-4 text-xs text-muted-foreground">
                  <div className="truncate">
                    Trigger: {w.trigger.repo_owner}/{w.trigger.repo_name}
                    {w.trigger.label ? ` · ${w.trigger.label}` : ""}
                  </div>
                  <WebhookHealthWarning
                    health={healthByInstall.get(w.trigger.install_id)}
                  />
                </CardContent>
              </Card>
            </Link>
          ))}
        </div>
      )}

      <WorkflowBuilderSheet open={creating} onOpenChange={setCreating} />
    </div>
  )
}
