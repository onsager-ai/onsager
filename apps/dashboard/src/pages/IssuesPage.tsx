import { useMemo, useState } from "react"
import { useQuery } from "@tanstack/react-query"
import { Inbox, RefreshCw } from "lucide-react"
import { Link, useSearchParams } from "react-router-dom"

import {
  api,
  type Project,
  type ProjectIssueRow,
  type SpineArtifact,
} from "@/lib/api"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table"
import { usePageHeader } from "@/components/layout/PageHeader"
import { BackfillDialog } from "@/components/BackfillDialog"
import { IssueActionsMenu } from "@/components/IssueActionsMenu"
import { useActiveWorkspace } from "@/lib/workspace"

type StateFilter = "open" | "closed" | "all"

/**
 * GitHub issues inbox (#168).
 *
 * Issues are reference-only `Kind::GithubIssue` artifacts (#167 / #170) —
 * the spine carries identity + our derived state, the proxy hydrates the
 * GitHub-authored fields (title, labels, assignees) on demand. This page
 * joins the two on `external_ref`.
 *
 * Workspace scope: the page accepts a `?project=` query param; without one,
 * it auto-selects the user's first project. Multi-project users get a
 * dropdown picker. Empty-state pushes the user back to `/workspaces` to
 * connect a project before there's anything to render.
 */
export function IssuesPage() {
  const workspace = useActiveWorkspace()

  // `?project=<id>` selects the active project on first render. Subsequent
  // user picks via the dropdown override into local state, so deep-links
  // stay clean while the picker still works.
  const [searchParams, setSearchParams] = useSearchParams()
  const urlProjectId = searchParams.get("project")
  const [projectIdOverride, setProjectIdOverride] = useState<string | null>(null)
  const [stateFilter, setStateFilter] = useState<StateFilter>("open")

  const projectsQuery = useQuery({
    queryKey: ["projects-for-user"],
    queryFn: api.listAllProjects,
  })
  const projects: Project[] = useMemo(() => {
    const all = projectsQuery.data?.projects ?? []
    return all.filter((p) => p.workspace_id === workspace.id)
  }, [projectsQuery.data, workspace.id])
  const selectedProjectId =
    projectIdOverride ?? urlProjectId ?? projects[0]?.id ?? null
  const selectedProject = useMemo(
    () => projects.find((p) => p.id === selectedProjectId) ?? null,
    [projects, selectedProjectId],
  )

  const setProjectId = (id: string) => {
    setProjectIdOverride(id)
    // Keep the URL in sync so the back/forward buttons + share-links work.
    const next = new URLSearchParams(searchParams)
    next.set("project", id)
    setSearchParams(next, { replace: true })
  }

  // Skeleton rows from the spine (kind=github_issue, scoped to project).
  // The hydrated fields come from the proxy below; we join on external_ref.
  const skeletonsQuery = useQuery({
    queryKey: ["artifacts", workspace.id, "github_issue", selectedProjectId],
    queryFn: () =>
      api.getArtifacts(workspace.id, {
        kind: "github_issue",
        project_id: selectedProjectId ?? undefined,
      }),
    enabled: !!selectedProjectId,
    refetchInterval: 15_000,
  })

  // Refetch cadence is independent of the server's proxy-cache TTL — the
  // server-side TTL bounds upstream-GitHub freshness; the client-side
  // interval bounds how often we *ask* the server. Picking 60s gives a
  // sensible refresh rate without being chatty; tuning it doesn't affect
  // correctness, only how quickly external changes appear without a
  // manual reload.
  const liveQuery = useQuery({
    queryKey: ["project-issues", selectedProjectId, stateFilter],
    queryFn: () => api.listProjectIssues(selectedProjectId!, stateFilter),
    enabled: !!selectedProjectId,
    refetchInterval: 60_000,
  })

  const skeletonsByExternalRef = useMemo(() => {
    const map = new Map<string, SpineArtifact>()
    for (const s of skeletonsQuery.data?.artifacts ?? []) {
      if (s.external_ref) map.set(s.external_ref, s)
    }
    return map
  }, [skeletonsQuery.data])

  const proxyError = liveQuery.data?.error ?? null

  // Drive the row set from the union of skeletons and live data so a proxy
  // failure (rate_limited / github_unreachable) renders the cached
  // skeleton rows rather than going blank — the page header banner already
  // explains the degraded state. Live fields hydrate over the skeleton
  // when both sides resolve.
  const rows: HydratedIssueRow[] = useMemo(() => {
    const live = liveQuery.data?.issues ?? []
    const liveByExternalRef = new Map<string, ProjectIssueRow>()
    if (selectedProjectId != null) {
      for (const issue of live) {
        liveByExternalRef.set(
          `github:project:${selectedProjectId}:issue:${issue.number}`,
          issue,
        )
      }
    }

    // Skeleton-rooted entries: every artifact we know about, hydrated when
    // possible.
    const skeletonRows: HydratedIssueRow[] = []
    for (const skeleton of skeletonsQuery.data?.artifacts ?? []) {
      const live =
        skeleton.external_ref != null
          ? (liveByExternalRef.get(skeleton.external_ref) ?? null)
          : null
      skeletonRows.push({ issue: live, skeleton })
    }

    // Live-only entries: webhook hasn't created a skeleton yet (e.g. the
    // very first time we observe an issue) but the proxy already returned
    // it. Suppress these when the proxy errored — those are the empty-list
    // case and we trust the skeletons alone.
    const liveOnlyRows: HydratedIssueRow[] = []
    if (!proxyError) {
      for (const issue of live) {
        if (selectedProjectId == null) {
          liveOnlyRows.push({ issue, skeleton: null })
          continue
        }
        const externalRef = `github:project:${selectedProjectId}:issue:${issue.number}`
        if (!skeletonsByExternalRef.has(externalRef)) {
          liveOnlyRows.push({ issue, skeleton: null })
        }
      }
    }

    return [...skeletonRows, ...liveOnlyRows]
  }, [
    liveQuery.data,
    proxyError,
    selectedProjectId,
    skeletonsByExternalRef,
    skeletonsQuery.data,
  ])

  usePageHeader({
    title: "Issues",
    actions:
      selectedProject != null ? (
        <BackfillDialog
          projectId={selectedProject.id}
          repoLabel={`${selectedProject.repo_owner}/${selectedProject.repo_name}`}
          trigger={
            <Button size="sm" variant="outline">
              <RefreshCw className="h-4 w-4" data-icon="inline-start" />
              <span className="hidden sm:inline">Backfill</span>
            </Button>
          }
        />
      ) : null,
  })

  if (projectsQuery.isLoading) {
    return <p className="py-8 text-center text-muted-foreground">Loading…</p>
  }

  if (projects.length === 0) {
    return (
      <div className="space-y-4 md:space-y-6">
        <h1 className="hidden text-2xl font-bold tracking-tight md:block">Issues</h1>
        <Card>
          <CardContent className="flex flex-col items-center gap-3 py-10 text-center">
            <Inbox className="h-8 w-8 text-muted-foreground" aria-hidden />
            <p className="text-sm text-muted-foreground">
              No projects connected. Set up a workspace to start ingesting issues.
            </p>
            <Button render={<Link to="/workspaces" />}>
              Go to workspaces
            </Button>
          </CardContent>
        </Card>
      </div>
    )
  }

  return (
    <div className="space-y-4 md:space-y-6">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div>
          <h1 className="hidden text-2xl font-bold tracking-tight md:block">Issues</h1>
          <p className="text-sm text-muted-foreground">
            GitHub issues ingested as factory artifacts.
          </p>
        </div>
        <div className="flex flex-wrap items-center gap-2">
          {projects.length > 1 ? (
            <Select
              value={selectedProjectId ?? undefined}
              onValueChange={(v) => {
                if (v) setProjectId(v)
              }}
            >
              <SelectTrigger className="w-56">
                <SelectValue placeholder="Project" />
              </SelectTrigger>
              <SelectContent>
                {projects.map((p) => (
                  <SelectItem key={p.id} value={p.id}>
                    {p.repo_owner}/{p.repo_name}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          ) : null}
          <Select
            value={stateFilter}
            onValueChange={(v) => setStateFilter(v as StateFilter)}
          >
            <SelectTrigger className="w-32">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="open">Open</SelectItem>
              <SelectItem value="closed">Closed</SelectItem>
              <SelectItem value="all">All</SelectItem>
            </SelectContent>
          </Select>
        </div>
      </div>

      {proxyError ? (
        <Card>
          <CardContent className="py-3 text-sm text-muted-foreground">
            {proxyError === "rate_limited"
              ? "GitHub rate limit reached. Showing skeleton rows; titles will return in about a minute."
              : "Couldn't reach GitHub. Showing the last-known artifacts; titles will refresh once the connection recovers."}
          </CardContent>
        </Card>
      ) : null}

      <Card>
        <CardHeader className="px-4 md:px-6">
          <CardTitle className="text-base md:text-lg">
            {selectedProject != null
              ? `${selectedProject.repo_owner}/${selectedProject.repo_name}`
              : "Issues"}
            {rows.length > 0 ? (
              <span className="font-normal text-muted-foreground">
                {" "}
                ({rows.length})
              </span>
            ) : null}
          </CardTitle>
        </CardHeader>
        <CardContent className="px-4 md:px-6">
          {liveQuery.isLoading ? (
            <p className="py-8 text-center text-muted-foreground">Loading…</p>
          ) : rows.length === 0 ? (
            <EmptyInbox project={selectedProject} />
          ) : (
            <>
              <div className="flex flex-col gap-2 md:hidden">
                {rows.map((r) => (
                  <IssueCard
                    key={rowKey(r)}
                    row={r}
                    projectId={selectedProjectId}
                    listQueryKey={[
                      "project-issues",
                      selectedProjectId,
                      stateFilter,
                    ]}
                  />
                ))}
              </div>
              <div className="hidden md:block">
                <Table>
                  <TableHeader>
                    <TableRow>
                      <TableHead>Title</TableHead>
                      <TableHead>State</TableHead>
                      <TableHead>Labels</TableHead>
                      <TableHead>Author</TableHead>
                      <TableHead>Updated</TableHead>
                      <TableHead></TableHead>
                    </TableRow>
                  </TableHeader>
                  <TableBody>
                    {rows.map((r) => {
                      const display = describeRow(r)
                      return (
                        <TableRow key={rowKey(r)}>
                          <TableCell className="max-w-md">
                            <div className="truncate font-medium">
                              {display.title}
                            </div>
                            <div className="text-xs text-muted-foreground">
                              {display.subtitle}
                            </div>
                          </TableCell>
                          <TableCell>
                            <Badge
                              variant={display.openState ? "default" : "secondary"}
                            >
                              {display.stateLabel}
                            </Badge>
                          </TableCell>
                          <TableCell>
                            <LabelChips labels={display.labels} />
                          </TableCell>
                          <TableCell className="text-muted-foreground">
                            {display.author}
                          </TableCell>
                          <TableCell className="text-xs text-muted-foreground">
                            {display.updatedAt}
                          </TableCell>
                          <TableCell>
                            <IssueActionsMenu
                              projectId={selectedProjectId}
                              issueNumber={r.issue?.number ?? null}
                              htmlUrl={display.htmlUrl}
                              listQueryKey={[
                                "project-issues",
                                selectedProjectId,
                                stateFilter,
                              ]}
                            />
                          </TableCell>
                        </TableRow>
                      )
                    })}
                  </TableBody>
                </Table>
              </div>
            </>
          )}
        </CardContent>
      </Card>
    </div>
  )
}

interface HydratedIssueRow {
  /// Live GitHub data, when the proxy returned it. NULL when the proxy
  /// failed open and we're rendering the skeleton row alone.
  issue: ProjectIssueRow | null
  /// Local skeleton row. NULL on the very first observation if the proxy
  /// returned an issue before the webhook fired.
  skeleton: SpineArtifact | null
}

interface RowDisplay {
  title: string
  subtitle: string
  openState: boolean
  stateLabel: string
  labels: string[]
  author: string
  updatedAt: string
  htmlUrl: string | null
}

/// Stable React key — uses the external_ref when available, otherwise
/// derives one from the live issue. Both sides shouldn't ever be null
/// for a row that survives the union, but the fallback keeps the type
/// system honest.
function rowKey(row: HydratedIssueRow): string {
  if (row.skeleton?.external_ref) return row.skeleton.external_ref
  if (row.issue) return `live:${row.issue.number}`
  return row.skeleton?.id ?? Math.random().toString(36)
}

/// Project hydrated + skeleton rows down to the values the table/card
/// renderers display. Skeleton-only rows derive their state from the
/// artifact's lifecycle column and surface a "—" placeholder for fields
/// only GitHub knows.
function describeRow(row: HydratedIssueRow): RowDisplay {
  const { issue, skeleton } = row
  if (issue) {
    return {
      title: issue.title,
      subtitle: `#${issue.number}`,
      openState: issue.state === "open",
      stateLabel: issue.state,
      labels: issue.labels,
      author: issue.author ?? "—",
      updatedAt: new Date(issue.updated_at).toLocaleString(),
      htmlUrl: issue.html_url,
    }
  }
  // Skeleton-only fallback: artifact lifecycle in `state` (`draft` =
  // open, `archived` = closed); titles aren't stored locally so we
  // surface the artifact id as a placeholder.
  const open = skeleton?.state === "draft"
  const lastSeen = skeleton?.last_observed_at
  return {
    title: skeleton?.id ?? "(unavailable)",
    subtitle: skeleton?.external_ref?.split(":issue:").pop() ?? "—",
    openState: open,
    stateLabel: open ? "open" : "closed",
    labels: [],
    author: "—",
    updatedAt: lastSeen
      ? `last seen ${new Date(lastSeen).toLocaleString()}`
      : "—",
    htmlUrl: null,
  }
}

function IssueCard({
  row,
  projectId,
  listQueryKey,
}: {
  row: HydratedIssueRow
  projectId: string | null
  listQueryKey: readonly unknown[]
}) {
  const display = describeRow(row)
  const cardClass =
    "flex flex-col gap-1.5 rounded-lg border p-3 transition-colors active:bg-accent"
  return (
    <div className={cardClass}>
      <div className="flex items-start justify-between gap-2">
        <span className="line-clamp-2 text-sm font-medium">{display.title}</span>
        <div className="flex shrink-0 items-center gap-1">
          <Badge variant={display.openState ? "default" : "secondary"}>
            {display.stateLabel}
          </Badge>
          <IssueActionsMenu
            projectId={projectId}
            issueNumber={row.issue?.number ?? null}
            htmlUrl={display.htmlUrl}
            listQueryKey={listQueryKey}
          />
        </div>
      </div>
      <div className="flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
        <span>{display.subtitle}</span>
        {display.author !== "—" ? <span>· {display.author}</span> : null}
        <span>· {display.updatedAt}</span>
      </div>
      {display.labels.length > 0 ? (
        <LabelChips labels={display.labels} />
      ) : null}
    </div>
  )
}

function LabelChips({ labels }: { labels: string[] }) {
  if (labels.length === 0) return null
  return (
    <div className="flex flex-wrap gap-1">
      {labels.slice(0, 3).map((l) => (
        <Badge key={l} variant="outline" className="text-xs">
          {l}
        </Badge>
      ))}
      {labels.length > 3 ? (
        <Badge variant="outline" className="text-xs">
          +{labels.length - 3}
        </Badge>
      ) : null}
    </div>
  )
}

function EmptyInbox({ project }: { project: Project | null }) {
  return (
    <div className="flex flex-col items-center gap-3 py-10 text-center">
      <Inbox className="h-8 w-8 text-muted-foreground" aria-hidden />
      <p className="text-sm text-muted-foreground">
        No issues in the inbox yet.
      </p>
      {project ? (
        <BackfillDialog
          projectId={project.id}
          repoLabel={`${project.repo_owner}/${project.repo_name}`}
          trigger={
            <Button>
              <RefreshCw className="h-4 w-4" data-icon="inline-start" />
              Backfill from GitHub
            </Button>
          }
        />
      ) : null}
    </div>
  )
}
