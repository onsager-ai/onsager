import { useMemo, useState } from "react"
import { useQuery } from "@tanstack/react-query"
import { ExternalLink, Inbox, RefreshCw } from "lucide-react"
import { Link } from "react-router-dom"

import {
  api,
  type Project,
  type ProjectIssueRow,
  type SpineArtifact,
} from "@/lib/api"
import { useAuth } from "@/lib/auth"
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
  const { authEnabled, user } = useAuth()
  const authed = !authEnabled || !!user

  const [projectId, setProjectId] = useState<string | null>(null)
  const [stateFilter, setStateFilter] = useState<StateFilter>("open")

  const projectsQuery = useQuery({
    queryKey: ["projects-for-user"],
    queryFn: api.listAllProjects,
    enabled: authed,
  })
  const projects: Project[] = useMemo(
    () => projectsQuery.data?.projects ?? [],
    [projectsQuery.data],
  )
  const selectedProjectId = projectId ?? projects[0]?.id ?? null
  const selectedProject = useMemo(
    () => projects.find((p) => p.id === selectedProjectId) ?? null,
    [projects, selectedProjectId],
  )

  // Skeleton rows from the spine (kind=github_issue, scoped to project).
  // The hydrated fields come from the proxy below; we join on external_ref.
  const skeletonsQuery = useQuery({
    queryKey: ["artifacts", "github_issue", selectedProjectId],
    queryFn: () =>
      api.getArtifacts({
        kind: "github_issue",
        project_id: selectedProjectId ?? undefined,
      }),
    enabled: authed && !!selectedProjectId,
    refetchInterval: 15_000,
  })

  const liveQuery = useQuery({
    queryKey: ["project-issues", selectedProjectId, stateFilter],
    queryFn: () => api.listProjectIssues(selectedProjectId!, stateFilter),
    enabled: authed && !!selectedProjectId,
    // Match the cache TTL on the server side; the dashboard re-fetches at
    // the same cadence so users always see something close to fresh.
    refetchInterval: 60_000,
  })

  const skeletonsByExternalRef = useMemo(() => {
    const map = new Map<string, SpineArtifact>()
    for (const s of skeletonsQuery.data?.artifacts ?? []) {
      if (s.external_ref) map.set(s.external_ref, s)
    }
    return map
  }, [skeletonsQuery.data])

  const rows: HydratedIssueRow[] = useMemo(() => {
    const live = liveQuery.data?.issues ?? []
    return live.map((issue) => {
      const externalRef =
        selectedProjectId != null
          ? `github:project:${selectedProjectId}:issue:${issue.number}`
          : null
      const skeleton =
        externalRef != null ? skeletonsByExternalRef.get(externalRef) : null
      return { issue, skeleton: skeleton ?? null }
    })
  }, [liveQuery.data, skeletonsByExternalRef, selectedProjectId])

  const proxyError = liveQuery.data?.error ?? null

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

  if (!authed) {
    return (
      <div className="space-y-4">
        <p className="text-muted-foreground">
          Sign in to see your project's GitHub issues.
        </p>
      </div>
    )
  }

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
              onValueChange={(v) => setProjectId(v)}
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

      {proxyError === "rate_limited" ? (
        <Card>
          <CardContent className="py-3 text-sm text-muted-foreground">
            GitHub rate limit reached. Showing skeleton rows; titles will return
            in about a minute.
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
                  <IssueCard key={r.issue.number} row={r} />
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
                    {rows.map(({ issue }) => (
                      <TableRow key={issue.number}>
                        <TableCell className="max-w-md">
                          <div className="truncate font-medium">{issue.title}</div>
                          <div className="text-xs text-muted-foreground">
                            #{issue.number}
                          </div>
                        </TableCell>
                        <TableCell>
                          <Badge
                            variant={issue.state === "open" ? "default" : "secondary"}
                          >
                            {issue.state}
                          </Badge>
                        </TableCell>
                        <TableCell>
                          <LabelChips labels={issue.labels} />
                        </TableCell>
                        <TableCell className="text-muted-foreground">
                          {issue.author ?? "—"}
                        </TableCell>
                        <TableCell className="text-xs text-muted-foreground">
                          {new Date(issue.updated_at).toLocaleString()}
                        </TableCell>
                        <TableCell>
                          <Button
                            size="sm"
                            variant="ghost"
                            render={
                              <a
                                href={issue.html_url}
                                target="_blank"
                                rel="noreferrer"
                              />
                            }
                          >
                            <ExternalLink className="h-3.5 w-3.5" />
                            <span className="sr-only">Open in GitHub</span>
                          </Button>
                        </TableCell>
                      </TableRow>
                    ))}
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
  issue: ProjectIssueRow
  skeleton: SpineArtifact | null
}

function IssueCard({ row }: { row: HydratedIssueRow }) {
  const { issue } = row
  return (
    <a
      href={issue.html_url}
      target="_blank"
      rel="noreferrer"
      className="flex flex-col gap-1.5 rounded-lg border p-3 transition-colors active:bg-accent"
    >
      <div className="flex items-start justify-between gap-2">
        <span className="line-clamp-2 text-sm font-medium">{issue.title}</span>
        <Badge
          variant={issue.state === "open" ? "default" : "secondary"}
          className="shrink-0"
        >
          {issue.state}
        </Badge>
      </div>
      <div className="flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
        <span>#{issue.number}</span>
        {issue.author ? <span>· {issue.author}</span> : null}
        <span>· {new Date(issue.updated_at).toLocaleDateString()}</span>
      </div>
      {issue.labels.length > 0 ? <LabelChips labels={issue.labels} /> : null}
    </a>
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
