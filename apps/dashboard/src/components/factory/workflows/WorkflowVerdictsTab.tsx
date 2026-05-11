import { useMemo } from "react"
import { useQuery } from "@tanstack/react-query"
import { Gavel } from "lucide-react"
import { api, type GovernanceEvent, type WorkflowRun } from "@/lib/api"
import { Badge } from "@/components/ui/badge"
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card"

const SEVERITY_VARIANT: Record<
  string,
  "destructive" | "default" | "secondary" | "outline"
> = {
  critical: "destructive",
  high: "destructive",
  medium: "default",
  low: "secondary",
}

// Does this governance event reference one of the workflow's artifact IDs?
// PR 2b replaces this with a workflow-scoped backend route; until then we
// pull the workspace-wide list and filter on whatever artifact-id reference
// the event happens to carry. Synodic events vary in shape, so we probe
// the common locations (`metadata.artifact_id`, `event.source`) and a
// last-ditch scan of metadata string values.
function referencesArtifact(
  event: GovernanceEvent,
  artifactIds: Set<string>,
): boolean {
  if (artifactIds.size === 0) return false
  const meta = event.metadata ?? {}
  const direct = meta["artifact_id"]
  if (typeof direct === "string" && artifactIds.has(direct)) return true
  if (event.source && artifactIds.has(event.source)) return true
  for (const value of Object.values(meta)) {
    if (typeof value === "string" && artifactIds.has(value)) return true
  }
  return false
}

export function WorkflowVerdictsTab({
  workspaceId,
  runs,
  stages,
}: {
  workspaceId: string
  runs: WorkflowRun[]
  stages: { gate_kind: string }[]
}) {
  const { data, isLoading } = useQuery({
    queryKey: ["governance-events", workspaceId],
    queryFn: () => api.getGovernanceEvents(workspaceId),
    refetchInterval: 5000,
  })

  const artifactIds = useMemo(
    () =>
      new Set(
        runs
          .map((r) => r.artifact_id)
          .filter((id): id is string => !!id),
      ),
    [runs],
  )

  const filtered = useMemo(
    () =>
      (data ?? []).filter((e) => referencesArtifact(e, artifactIds)),
    [data, artifactIds],
  )

  const hasGovernanceStage = stages.some(
    (s) => s.gate_kind === "governance",
  )

  if (!hasGovernanceStage) {
    return (
      <EmptyState
        title="No governance-gated stages"
        body="This workflow has no governance gates, so no verdicts will appear here."
      />
    )
  }

  if (isLoading) {
    return (
      <Card>
        <CardContent className="px-4 py-6 md:px-6">
          <p className="text-center text-sm text-muted-foreground">Loading…</p>
        </CardContent>
      </Card>
    )
  }

  if (filtered.length === 0) {
    return (
      <EmptyState
        title="No verdicts yet"
        body="Verdicts appear once a governance gate evaluates an artifact from this workflow."
      />
    )
  }

  return (
    <Card>
      <CardHeader className="px-4 pb-2 pt-4 md:px-6">
        <CardTitle className="text-base">Verdicts</CardTitle>
      </CardHeader>
      <CardContent className="space-y-2 px-4 pb-4 md:px-6">
        {filtered.map((e) => (
          <div
            key={e.id}
            className="space-y-1 rounded-md border px-3 py-2"
          >
            <div className="flex items-center gap-2">
              <Badge variant={SEVERITY_VARIANT[e.severity] ?? "outline"}>
                {e.severity}
              </Badge>
              <Badge variant="outline" className="font-mono text-[10px]">
                {e.event_type}
              </Badge>
              <span className="ml-auto shrink-0 text-xs text-muted-foreground">
                {new Date(e.created_at).toLocaleString()}
              </span>
            </div>
            <p className="text-sm">{e.title}</p>
            {e.resolved && (
              <p className="text-xs text-muted-foreground">
                resolved
                {e.resolution_notes ? ` · ${e.resolution_notes}` : ""}
              </p>
            )}
          </div>
        ))}
      </CardContent>
    </Card>
  )
}

function EmptyState({ title, body }: { title: string; body: string }) {
  return (
    <Card>
      <CardContent className="flex flex-col items-center gap-3 py-10 text-center">
        <div className="flex h-12 w-12 items-center justify-center rounded-full bg-muted text-muted-foreground">
          <Gavel className="h-6 w-6" />
        </div>
        <div className="space-y-1">
          <p className="text-base font-medium">{title}</p>
          <p className="text-sm text-muted-foreground">{body}</p>
        </div>
      </CardContent>
    </Card>
  )
}
