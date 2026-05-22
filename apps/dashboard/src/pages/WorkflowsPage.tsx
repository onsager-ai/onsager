import { useMemo, useState } from "react"
import { Link, useSearchParams } from "react-router-dom"
import { useQuery } from "@tanstack/react-query"
import { GitBranch, Plus } from "lucide-react"
import { api, type WorkflowStatusChip } from "@/lib/api"
import { useActiveWorkspace } from "@/lib/workspace"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card"
import { ActiveRunsBanner } from "@/components/workflows/ActiveRunsBanner"
import { WebhookHealthWarning } from "@/components/workflows/WebhookHealthWarning"
import { WorkflowBuilderSheet } from "@/components/workflows/WorkflowBuilderSheet"
import { WorkflowStatusChips } from "@/components/workflows/WorkflowStatusChips"
import { usePageHeader } from "@/components/layout/PageHeader"

const VALID_CHIPS: WorkflowStatusChip[] = [
  "all",
  "drafted",
  "bound",
  "active",
  "paused",
]

function readChipFromQuery(raw: string | null): WorkflowStatusChip {
  if (!raw) return "all"
  return (VALID_CHIPS as readonly string[]).includes(raw)
    ? (raw as WorkflowStatusChip)
    : "all"
}

export function WorkflowsPage() {
  usePageHeader({ title: "Workflows" })
  const workspace = useActiveWorkspace()
  const [creating, setCreating] = useState(false)
  const [params, setParams] = useSearchParams()

  const chip = readChipFromQuery(params.get("state"))

  const { data, isLoading } = useQuery({
    queryKey: ["workflows", workspace.id, chip],
    queryFn: () => api.listWorkflows(workspace.id, chip),
  })
  const workflows = useMemo(() => data?.workflows ?? [], [data])
  const counts = data?.counts ?? {
    drafted: 0,
    bound: 0,
    active: 0,
    paused: 0,
    all: 0,
  }

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

  const setChip = (next: WorkflowStatusChip) => {
    const updated = new URLSearchParams(params)
    if (next === "all") updated.delete("state")
    else updated.set("state", next)
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

      <WorkflowStatusChips value={chip} counts={counts} onChange={setChip} />

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
                {chip === "all"
                  ? "No workflows yet"
                  : `No ${chip} workflows`}
              </p>
              <p className="text-sm text-muted-foreground">
                {chip === "all"
                  ? "Set one up and Onsager starts responding to GitHub events. Open ⌘K to create one."
                  : "Adjust the filter above or create a workflow via ⌘K."}
              </p>
            </div>
            {chip === "all" && (
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
