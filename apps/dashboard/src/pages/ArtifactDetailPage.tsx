import { useParams, Link } from "react-router-dom"
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query"
import { useState } from "react"
import { api, ApiError, type OverrideGateRequestBody } from "@/lib/api"
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card"
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
import { ArrowLeft, Ban, MoreHorizontal, RefreshCw, ShieldCheck } from "lucide-react"
import { LineageDAG } from "@/components/factory/LineageDAG"
import { usePageHeader } from "@/components/layout/PageHeader"

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
  const queryClient = useQueryClient()
  const [banner, setBanner] = useState<ActionBanner>(null)

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

  const overrideMutation = useMutation({
    mutationFn: (body: OverrideGateRequestBody) => api.overrideGate(id!, body),
    onSuccess: (res) => {
      setBanner({
        kind: "ok",
        message: `Gate override emitted (${res.verdict ?? "allow"}).`,
      })
      invalidate()
    },
    onError: (err) => showError("Override failed", err),
  })

  const handleAbort = () => {
    const reason = window.prompt(
      "Reason for aborting this artifact?",
      "aborted via dashboard",
    )
    if (reason === null) return
    abortMutation.mutate(reason.trim() || "aborted via dashboard")
  }

  const handleOverride = (verdict: "allow" | "deny") => {
    const reason = window.prompt(
      `Reason for gate ${verdict}?`,
      `manual ${verdict} via dashboard`,
    )
    if (reason === null) return
    overrideMutation.mutate({
      verdict,
      reason: reason.trim() || `manual ${verdict} via dashboard`,
      actor: "dashboard",
    })
  }

  const isTerminal =
    artifact?.state === "archived" || artifact?.state === "released"
  const archived = artifact?.state === "archived"
  const busy =
    retryMutation.isPending ||
    abortMutation.isPending ||
    overrideMutation.isPending

  // Mobile chrome: back + artifact name + overflow menu (Retry / Override
  // / Abort). Desktop renders the same 4 actions inline below.
  usePageHeader({
    title: artifact?.name ?? "Artifact",
    backTo: "/artifacts",
    actions: artifact ? (
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
            disabled={busy || archived}
            onClick={() => handleOverride("allow")}
          >
            <ShieldCheck className="mr-2 h-4 w-4" />
            Override gate: Allow
          </DropdownMenuItem>
          <DropdownMenuItem
            disabled={busy || archived}
            onClick={() => handleOverride("deny")}
          >
            <ShieldCheck className="mr-2 h-4 w-4" />
            Override gate: Deny
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
        to="/artifacts"
        className="hidden items-center gap-1 text-sm text-muted-foreground hover:text-foreground md:inline-flex"
      >
        <ArrowLeft className="h-4 w-4" /> Back to Artifacts
      </Link>

      <div className="flex items-start justify-between gap-4">
        <div className="min-w-0">
          <h1 className="hidden truncate text-2xl font-bold tracking-tight md:block">
            {artifact.name}
          </h1>
          <p className="truncate text-sm text-muted-foreground font-mono">
            {artifact.id}
          </p>
        </div>
        <Badge variant={STATE_VARIANT[artifact.state] || "secondary"} className="text-sm">
          {artifact.state.replace("_", " ")}
        </Badge>
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
              : "Request another shaping run"
          }
        >
          <RefreshCw className="mr-1 h-3.5 w-3.5" />
          Retry
        </Button>
        <Button
          variant="outline"
          size="sm"
          disabled={busy || archived}
          onClick={() => handleOverride("allow")}
          title="Manually allow an escalated gate"
        >
          <ShieldCheck className="mr-1 h-3.5 w-3.5" />
          Override gate: Allow
        </Button>
        <Button
          variant="outline"
          size="sm"
          disabled={busy || archived}
          onClick={() => handleOverride("deny")}
        >
          <ShieldCheck className="mr-1 h-3.5 w-3.5" />
          Override: Deny
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

      {/* Metadata */}
      <div className="grid grid-cols-2 gap-3 md:grid-cols-4">
        <Card>
          <CardContent className="px-3 py-3">
            <div className="text-xs text-muted-foreground">Kind</div>
            <div className="font-medium">{artifact.kind}</div>
          </CardContent>
        </Card>
        <Card>
          <CardContent className="px-3 py-3">
            <div className="text-xs text-muted-foreground">Owner</div>
            <div className="font-medium">{artifact.owner}</div>
          </CardContent>
        </Card>
        <Card>
          <CardContent className="px-3 py-3">
            <div className="text-xs text-muted-foreground">Version</div>
            <div className="font-mono font-medium">v{artifact.current_version}</div>
          </CardContent>
        </Card>
        <Card>
          <CardContent className="px-3 py-3">
            <div className="text-xs text-muted-foreground">Created</div>
            <div className="text-sm">{new Date(artifact.created_at).toLocaleDateString()}</div>
          </CardContent>
        </Card>
      </div>

      {/* Per-run lineage DAG */}
      <Card>
        <CardHeader className="px-4 md:px-6">
          <CardTitle className="text-base md:text-lg">Run Lineage</CardTitle>
        </CardHeader>
        <CardContent className="px-4 md:px-6">
          <LineageDAG artifact={artifact} />
        </CardContent>
      </Card>

      {/* Version History */}
      <Card>
        <CardHeader className="px-4 md:px-6">
          <CardTitle className="text-base md:text-lg">
            Version History
            {artifact.versions && artifact.versions.length > 0 && (
              <span className="ml-2 text-muted-foreground font-normal">
                ({artifact.versions.length})
              </span>
            )}
          </CardTitle>
        </CardHeader>
        <CardContent className="px-4 md:px-6">
          {!artifact.versions || artifact.versions.length === 0 ? (
            <p className="py-4 text-center text-sm text-muted-foreground">
              No versions yet. Versions are created as Forge shapes this artifact.
            </p>
          ) : (
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>Version</TableHead>
                  <TableHead>Summary</TableHead>
                  <TableHead>Session</TableHead>
                  <TableHead>Created</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {[...artifact.versions]
                  .sort((a, b) => b.version - a.version)
                  .map((v) => (
                    <TableRow key={v.version}>
                      <TableCell className="font-mono">v{v.version}</TableCell>
                      <TableCell className="max-w-[300px] truncate">
                        {v.change_summary || "-"}
                      </TableCell>
                      <TableCell>
                        <Link
                          to={`/sessions/${v.created_by_session}`}
                          className="font-mono text-xs hover:underline"
                        >
                          {v.created_by_session.slice(0, 8)}
                        </Link>
                      </TableCell>
                      <TableCell className="text-xs text-muted-foreground">
                        {new Date(v.created_at).toLocaleString()}
                      </TableCell>
                    </TableRow>
                  ))}
              </TableBody>
            </Table>
          )}
        </CardContent>
      </Card>

      {/* Lineage */}
      {artifact.vertical_lineage && artifact.vertical_lineage.length > 0 && (
        <Card>
          <CardHeader className="px-4 md:px-6">
            <CardTitle className="text-base md:text-lg">Vertical Lineage</CardTitle>
          </CardHeader>
          <CardContent className="px-4 md:px-6">
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>Version</TableHead>
                  <TableHead>Session</TableHead>
                  <TableHead>Recorded</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {artifact.vertical_lineage.map((entry, i) => (
                  <TableRow key={i}>
                    <TableCell className="font-mono">v{entry.version}</TableCell>
                    <TableCell>
                      <Link
                        to={`/sessions/${entry.session_id}`}
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
          </CardContent>
        </Card>
      )}

      {artifact.horizontal_lineage && artifact.horizontal_lineage.length > 0 && (
        <Card>
          <CardHeader className="px-4 md:px-6">
            <CardTitle className="text-base md:text-lg">Horizontal Lineage</CardTitle>
          </CardHeader>
          <CardContent className="px-4 md:px-6">
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>Role</TableHead>
                  <TableHead>Source Artifact</TableHead>
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
                        to={`/artifacts/${entry.source_artifact_id}`}
                        className="font-mono text-xs hover:underline"
                      >
                        {entry.source_artifact_id}
                      </Link>
                    </TableCell>
                    <TableCell className="font-mono">v{entry.source_version}</TableCell>
                    <TableCell className="text-xs text-muted-foreground">
                      {new Date(entry.recorded_at).toLocaleString()}
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </CardContent>
        </Card>
      )}
    </div>
  )
}
