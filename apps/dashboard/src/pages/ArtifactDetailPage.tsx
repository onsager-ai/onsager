import { useParams, useSearchParams, Link } from "react-router-dom"
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query"
import { useMemo, useState } from "react"
import { api, ApiError } from "@/lib/api"
import type { ArtifactDetail } from "@/lib/api/generated/ArtifactDetail"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table"
import { ArrowLeft, Ban, MoreHorizontal, RefreshCw } from "lucide-react"
import { LineageDAG } from "@/components/workflows/LineageDAG"
import { ArtifactContent } from "@/components/artifacts/ArtifactContent"
import { ChatAskButton } from "@/components/chat/ChatAskButton"
import { usePageHeader } from "@/components/layout/PageHeader"
import { useActiveWorkspace } from "@/lib/workspace"

// Provenance axes (ADR 0021 inv. 3 — tabs/segmented views are allowed
// for orthogonal views of the *same* data). Time is the default and
// absorbs the former Version History card.
const PROVENANCE_AXES = [
  { key: "time", label: "Time" },
  { key: "origin", label: "Origin" },
  { key: "relations", label: "Relations" },
] as const
type ProvenanceAxis = (typeof PROVENANCE_AXES)[number]["key"]

function readAxis(raw: string | null): ProvenanceAxis {
  return PROVENANCE_AXES.some((a) => a.key === raw)
    ? (raw as ProvenanceAxis)
    : "time"
}

const STATE_VARIANT: Record<string, "default" | "secondary" | "destructive" | "outline"> = {
  draft: "outline",
  in_progress: "default",
  under_review: "secondary",
  released: "default",
  archived: "secondary",
}

type ActionBanner =
  | { kind: "ok"; message: string }
  | { kind: "error"; message: string }
  | null

export function ArtifactDetailPage() {
  const { id } = useParams<{ id: string }>()
  const workspace = useActiveWorkspace()
  const queryClient = useQueryClient()
  const [banner, setBanner] = useState<ActionBanner>(null)
  const [params, setParams] = useSearchParams()
  const axis = readAxis(params.get("axis"))
  const setAxis = (next: ProvenanceAxis) => {
    const updated = new URLSearchParams(params)
    if (next === "time") updated.delete("axis")
    else updated.set("axis", next)
    setParams(updated, { replace: true })
  }

  const { data, isLoading, error } = useQuery({
    queryKey: ["artifact", id],
    queryFn: () => api.getArtifact(id!),
    enabled: !!id,
    refetchInterval: 5000,
  })

  const artifact = data?.artifact

  const invalidate = () =>
    queryClient.invalidateQueries({ queryKey: ["artifact", id] })

  const showError = (label: string, err: unknown) => {
    const message =
      err instanceof ApiError
        ? err.message
        : err instanceof Error
          ? err.message
          : "unknown error"
    setBanner({ kind: "error", message: `${label}: ${message}` })
  }

  const retryMutation = useMutation({
    mutationFn: () => api.retryArtifact(id!, { actor: "dashboard" }),
    onSuccess: () => {
      setBanner({ kind: "ok", message: "Retry requested." })
      invalidate()
    },
    onError: (err) => showError("Retry failed", err),
  })

  const abortMutation = useMutation({
    mutationFn: (reason: string) =>
      api.abortArtifact(id!, { reason, actor: "dashboard" }),
    onSuccess: () => {
      setBanner({ kind: "ok", message: "Artifact archived." })
      invalidate()
    },
    onError: (err) => showError("Abort failed", err),
  })

  const handleAbort = () => {
    const reason = window.prompt(
      "Reason for aborting this artifact?",
      "aborted via dashboard",
    )
    if (reason === null) return
    abortMutation.mutate(reason.trim() || "aborted via dashboard")
  }

  const isTerminal =
    artifact?.state === "archived" || artifact?.state === "released"
  const archived = artifact?.state === "archived"
  const busy = retryMutation.isPending || abortMutation.isPending

  // Mobile chrome: back + artifact name + ⋯ overflow menu. Desktop
  // renders the same 4 actions as inline buttons below. Memoized per
  // the dashboard-ui rule on JSX `actions`.
  const headerActions = useMemo(
    () =>
      artifact ? (
        <DropdownMenu>
          <DropdownMenuTrigger
            render={
              <Button
                variant="ghost"
                size="icon"
                className="h-9 w-9"
                aria-label="Artifact actions"
              />
            }
          >
            <MoreHorizontal className="h-5 w-5" />
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end" className="w-52">
            <DropdownMenuItem
              disabled={busy || isTerminal}
              onClick={() => retryMutation.mutate()}
            >
              <RefreshCw className="mr-2 h-4 w-4" />
              Retry
            </DropdownMenuItem>
            <DropdownMenuItem
              variant="destructive"
              disabled={busy || archived}
              onClick={handleAbort}
            >
              <Ban className="mr-2 h-4 w-4" />
              Abort
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      ) : null,
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [artifact, busy, isTerminal, archived],
  )
  usePageHeader({
    title: artifact?.name ?? "Artifact",
    backTo: `/workspaces/${workspace.slug}/artifacts`,
    actions: headerActions,
  })

  if (isLoading) {
    return (
      <div className="flex min-h-[200px] items-center justify-center">
        <p className="text-muted-foreground">Loading...</p>
      </div>
    )
  }

  if (error || !artifact) {
    return (
      <div className="space-y-4">
        <p className="text-destructive">Artifact not found.</p>
      </div>
    )
  }

  return (
    <div className="space-y-4 md:space-y-6">
      {/* Desktop header — back link + title + state badge. Mobile uses
          the global top bar (back arrow + title + overflow menu). */}
      <Link
        to={`/workspaces/${workspace.slug}/artifacts`}
        className="hidden items-center gap-1 text-sm text-muted-foreground hover:text-foreground md:inline-flex"
      >
        <ArrowLeft className="h-4 w-4" /> Back to Artifacts
      </Link>

      <div className="flex items-start justify-between gap-4">
        <div className="min-w-0">
          <h1 className="hidden truncate text-2xl font-bold tracking-tight md:block">
            {artifact.name ?? artifact.id}
          </h1>
          <p className="truncate text-sm text-muted-foreground font-mono">
            {artifact.id}
          </p>
        </div>
        <div className="flex items-center gap-2">
          <ChatAskButton
            scope={{ type: "workspace", id: null }}
            label="Ask about this artifact"
          />
          <Badge variant={STATE_VARIANT[artifact.state] || "secondary"} className="text-sm">
            {artifact.state.replace("_", " ")}
          </Badge>
        </div>
      </div>

      {banner && (
        <div
          className={
            banner.kind === "ok"
              ? "rounded-md border border-emerald-300 bg-emerald-50 px-3 py-2 text-sm text-emerald-900 dark:border-emerald-900 dark:bg-emerald-950/40 dark:text-emerald-200"
              : "rounded-md border border-red-300 bg-red-50 px-3 py-2 text-sm text-red-900 dark:border-red-900 dark:bg-red-950/40 dark:text-red-200"
          }
          role="status"
        >
          {banner.message}
        </div>
      )}

      {/* Desktop controls — mobile uses the overflow menu in the global
          top bar via usePageHeader above. */}
      <div className="hidden flex-wrap gap-2 md:flex">
        <Button
          variant="outline"
          size="sm"
          disabled={busy || isTerminal}
          onClick={() => retryMutation.mutate()}
          title={
            isTerminal
              ? "Cannot retry an artifact in a terminal state"
              : "Request another run"
          }
        >
          <RefreshCw className="mr-1 h-3.5 w-3.5" />
          Retry
        </Button>
        <Button
          variant="destructive"
          size="sm"
          disabled={busy || archived}
          onClick={handleAbort}
        >
          <Ban className="mr-1 h-3.5 w-3.5" />
          Abort
        </Button>
      </div>

      {/* Single-value metadata absorbed into the header strip (ADR 0021
          inv. 4) — replaces the former 4-up metadata grid and the
          "Onsager metadata" Card from IssueDetail (#492). Lifecycle is
          an artifact state-machine attribute, applies to every kind. */}
      <div className="grid grid-cols-2 gap-x-4 gap-y-2 rounded-md border bg-muted/30 px-3 py-2 text-sm sm:grid-cols-3 lg:grid-cols-6">
        <Meta label="Kind">{artifact.kind}</Meta>
        <Meta label="Lifecycle">{artifact.state.replaceAll("_", " ")}</Meta>
        <Meta label="Owner">{artifact.owner ?? "—"}</Meta>
        <Meta label="Version">
          <span className="font-mono">v{artifact.current_version}</span>
        </Meta>
        <Meta label="Created">
          {new Date(artifact.created_at).toLocaleDateString()}
        </Meta>
        <Meta label="Last observed">
          {artifact.last_observed_at
            ? new Date(artifact.last_observed_at).toLocaleString()
            : "—"}
        </Meta>
      </div>

      {/* Content — dispatched by artifact_kind (#491). */}
      <section className="space-y-3">
        <h2 className="text-lg font-semibold tracking-tight">Content</h2>
        <ArtifactContent artifact={artifact} />
      </section>

      {/* Provenance — one graph, three axes (ADR 0021). Axis is mirrored
          to `?axis=` for bookmark stability. */}
      <section className="space-y-3">
        <div className="flex flex-wrap items-center justify-between gap-2">
          <h2 className="text-lg font-semibold tracking-tight">Provenance</h2>
          <div className="flex items-center gap-0.5 rounded-md border p-0.5">
            {PROVENANCE_AXES.map((a) => (
              <Button
                key={a.key}
                type="button"
                size="sm"
                variant={axis === a.key ? "default" : "ghost"}
                className="h-7 px-3 text-xs"
                onClick={() => setAxis(a.key)}
              >
                {a.label}
              </Button>
            ))}
          </div>
        </div>
        {axis === "time" && (
          <ProvenanceTime artifact={artifact} workspaceSlug={workspace.slug} />
        )}
        {axis === "origin" && (
          <ProvenanceOrigin artifact={artifact} workspaceSlug={workspace.slug} />
        )}
        {axis === "relations" && (
          <ProvenanceRelations
            artifact={artifact}
            workspaceSlug={workspace.slug}
          />
        )}
      </section>

      {/* Consumers — reverse traceability (ADR 0021): the runs/stages
          that consumed this artifact as input. */}
      <section className="space-y-3">
        <h2 className="text-lg font-semibold tracking-tight">Consumers</h2>
        <ConsumersSection artifact={artifact} workspaceSlug={workspace.slug} />
      </section>
    </div>
  )
}

function Meta({
  label,
  children,
}: {
  label: string
  children: React.ReactNode
}) {
  return (
    <div className="min-w-0 space-y-0.5">
      <div className="text-[10px] uppercase tracking-wide text-muted-foreground">
        {label}
      </div>
      <div className="truncate">{children}</div>
    </div>
  )
}

// Time axis — the version-sequence walk. Absorbs the former Version
// History card; each row carries an Ask button (ADR 0021 adoption
// checklist) and a SourceTag slot wired in #488.
function ProvenanceTime({
  artifact,
  workspaceSlug,
}: {
  artifact: ArtifactDetail
  workspaceSlug: string
}) {
  if (!artifact.versions || artifact.versions.length === 0) {
    return (
      <p className="py-4 text-center text-sm text-muted-foreground">
        No versions yet. Versions are created as Forge shapes this artifact.
      </p>
    )
  }
  const versions = [...artifact.versions].sort((a, b) => b.version - a.version)
  return (
    <div className="overflow-x-auto rounded-md border">
      <Table>
        <TableHeader>
          <TableRow>
            <TableHead>Version</TableHead>
            <TableHead>Summary</TableHead>
            <TableHead>Source</TableHead>
            <TableHead>Session</TableHead>
            <TableHead>Created</TableHead>
            <TableHead className="text-right">Ask</TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          {versions.map((v) => (
            <TableRow key={v.version}>
              <TableCell className="font-mono">v{v.version}</TableCell>
              <TableCell className="max-w-[260px] truncate">
                {v.change_summary || "-"}
              </TableCell>
              <TableCell className="text-xs text-muted-foreground">
                {/* SourceTag (#488) */}—
              </TableCell>
              <TableCell>
                <Link
                  to={`/workspaces/${workspaceSlug}/sessions/${v.created_by_session}`}
                  className="font-mono text-xs hover:underline"
                >
                  {v.created_by_session.slice(0, 8)}
                </Link>
              </TableCell>
              <TableCell className="text-xs text-muted-foreground">
                {new Date(v.created_at).toLocaleString()}
              </TableCell>
              <TableCell className="text-right">
                <ChatAskButton
                  scope={{ type: "workspace", id: null }}
                  label="Ask"
                  variant="ghost"
                  size="icon"
                />
              </TableCell>
            </TableRow>
          ))}
        </TableBody>
      </Table>
    </div>
  )
}

// Origin axis — where the artifact came from: the producing run's
// internal DAG plus the per-version session that shaped it.
function ProvenanceOrigin({
  artifact,
  workspaceSlug,
}: {
  artifact: ArtifactDetail
  workspaceSlug: string
}) {
  return (
    <div className="space-y-4">
      <LineageDAG artifact={artifact} />
      {artifact.vertical_lineage && artifact.vertical_lineage.length > 0 ? (
        <div className="overflow-x-auto rounded-md border">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Version</TableHead>
                <TableHead>Source</TableHead>
                <TableHead>Session</TableHead>
                <TableHead>Recorded</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {artifact.vertical_lineage.map((entry, i) => (
                <TableRow key={i}>
                  <TableCell className="font-mono">v{entry.version}</TableCell>
                  <TableCell className="text-xs text-muted-foreground">
                    {/* SourceTag (#488) */}—
                  </TableCell>
                  <TableCell>
                    <Link
                      to={`/workspaces/${workspaceSlug}/sessions/${entry.session_id}`}
                      className="font-mono text-xs hover:underline"
                    >
                      {entry.session_id.slice(0, 8)}
                    </Link>
                  </TableCell>
                  <TableCell className="text-xs text-muted-foreground">
                    {new Date(entry.recorded_at).toLocaleString()}
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        </div>
      ) : null}
    </div>
  )
}

// Relations axis — sibling artifacts wired into this one's production
// (the former Horizontal Lineage card).
function ProvenanceRelations({
  artifact,
  workspaceSlug,
}: {
  artifact: ArtifactDetail
  workspaceSlug: string
}) {
  if (!artifact.horizontal_lineage || artifact.horizontal_lineage.length === 0) {
    return (
      <p className="py-4 text-center text-sm text-muted-foreground">
        No related artifacts recorded for this run.
      </p>
    )
  }
  return (
    <div className="overflow-x-auto rounded-md border">
      <Table>
        <TableHeader>
          <TableRow>
            <TableHead>Role</TableHead>
            <TableHead>Source Artifact</TableHead>
            <TableHead>Source</TableHead>
            <TableHead>Version</TableHead>
            <TableHead>Recorded</TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          {artifact.horizontal_lineage.map((entry) => (
            <TableRow
              key={`${entry.role}-${entry.source_artifact_id}-${entry.source_version}-${entry.recorded_at}`}
            >
              <TableCell className="text-xs">{entry.role}</TableCell>
              <TableCell>
                <Link
                  to={`/workspaces/${workspaceSlug}/artifacts/${entry.source_artifact_id}`}
                  className="font-mono text-xs hover:underline"
                >
                  {entry.source_artifact_id}
                </Link>
              </TableCell>
              <TableCell className="text-xs text-muted-foreground">
                {/* SourceTag (#488) */}—
              </TableCell>
              <TableCell className="font-mono">v{entry.source_version}</TableCell>
              <TableCell className="text-xs text-muted-foreground">
                {new Date(entry.recorded_at).toLocaleString()}
              </TableCell>
            </TableRow>
          ))}
        </TableBody>
      </Table>
    </div>
  )
}

interface ConsumerRef {
  artifact_id?: string
  run_id?: string
  stage?: string
}

// Consumers — reads the spine `artifacts.consumers` array. The column is
// populated by a future substrate writer (defaults to `[]` today), so
// the section reads the real field and shows the empty state until then
// rather than fabricating reverse edges.
function ConsumersSection({
  artifact,
  workspaceSlug,
}: {
  artifact: ArtifactDetail
  workspaceSlug: string
}) {
  const consumers: ConsumerRef[] = Array.isArray(artifact.consumers)
    ? (artifact.consumers as ConsumerRef[])
    : []
  if (consumers.length === 0) {
    return (
      <p className="py-4 text-center text-sm text-muted-foreground">
        Nothing has consumed this artifact yet.
      </p>
    )
  }
  const INLINE = 10
  const visible = consumers.slice(0, INLINE)
  const activityHref = `/workspaces/${workspaceSlug}/activity?artifact=${encodeURIComponent(
    artifact.id,
  )}`
  return (
    <div className="space-y-2">
      {visible.map((c, i) => {
        const target = c.run_id
          ? `/workspaces/${workspaceSlug}/runs/${c.run_id}`
          : c.artifact_id
            ? `/workspaces/${workspaceSlug}/artifacts/${c.artifact_id}`
            : null
        const label = c.run_id ?? c.artifact_id ?? "consumer"
        return (
          <div
            key={`${label}-${i}`}
            className="flex items-center justify-between gap-2 rounded-md border px-3 py-2"
          >
            <div className="flex min-w-0 items-center gap-2">
              {/* SourceTag (#488) */}
              {target ? (
                <Link to={target} className="truncate font-mono text-xs hover:underline">
                  {label}
                </Link>
              ) : (
                <span className="truncate font-mono text-xs">{label}</span>
              )}
              {c.stage ? (
                <Badge variant="outline" className="shrink-0 text-[10px]">
                  {c.stage}
                </Badge>
              ) : null}
            </div>
            <ChatAskButton
              scope={{ type: "workspace", id: null }}
              label="Ask"
              variant="ghost"
              size="icon"
            />
          </div>
        )
      })}
      {consumers.length > INLINE && (
        <Button variant="outline" size="sm" render={<Link to={activityHref} />}>
          {consumers.length - INLINE} more in Activity →
        </Button>
      )}
    </div>
  )
}
