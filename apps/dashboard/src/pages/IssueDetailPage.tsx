import { useMemo } from "react"
import { useQuery } from "@tanstack/react-query"
import { Link, useParams } from "react-router-dom"
import { ArrowLeft, ExternalLink } from "lucide-react"

import {
  api,
  ApiError,
  type ProjectIssueDetail,
  type SpineArtifact,
} from "@/lib/api"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card"
import { IssueActionsMenu } from "@/components/IssueActionsMenu"
import { usePageHeader } from "@/components/layout/PageHeader"
import { useActiveWorkspace } from "@/lib/workspace"

/**
 * Issue detail page (#205).
 *
 * Joins the live-hydrated GitHub issue (`GET /api/projects/:id/issues/:n`)
 * with the matching reference-only `SpineArtifact` skeleton (kind=
 * `github_issue`, looked up by `external_ref`). The proxy is fail-open
 * per #170: if GitHub is rate-limited or unreachable, the live response
 * carries `{ issue: null, error: "..." }` and we render the skeleton
 * alone with the same banner copy as the inbox.
 */
export function IssueDetailPage() {
  const workspace = useActiveWorkspace()
  const { projectId = "", number: numberParam = "" } = useParams<{
    projectId: string
    number: string
  }>()
  // Strict digit-only check before parseInt — a malformed segment like
  // "42abc" would otherwise parse to 42 and silently link to the wrong
  // issue. queryKey uses the raw string so an invalid input never
  // serializes NaN into the cache key.
  const numberValid = /^\d+$/.test(numberParam)
  const issueNumber = numberValid ? Number.parseInt(numberParam, 10) : 0

  const liveQuery = useQuery({
    queryKey: ["project-issue", projectId, numberParam],
    queryFn: () => api.getProjectIssue(projectId, issueNumber),
    enabled: !!projectId && numberValid,
    refetchInterval: 60_000,
  })

  // Skeleton lookup: filter the workspace's `github_issue` artifacts by
  // project, then match the row whose `external_ref` matches this issue.
  // Same join key the inbox uses.
  const skeletonsQuery = useQuery({
    queryKey: ["artifacts", workspace.id, "github_issue", projectId],
    queryFn: () =>
      api.getArtifacts(workspace.id, {
        kind: "github_issue",
        project_id: projectId,
      }),
    enabled: !!projectId && numberValid,
    refetchInterval: 30_000,
  })

  const skeleton: SpineArtifact | null = useMemo(() => {
    if (!projectId || !numberValid) return null
    const externalRef = `github:project:${projectId}:issue:${issueNumber}`
    return (
      skeletonsQuery.data?.artifacts.find(
        (a) => a.external_ref === externalRef,
      ) ?? null
    )
  }, [skeletonsQuery.data, projectId, issueNumber, numberValid])

  const issue: ProjectIssueDetail | null = liveQuery.data?.issue ?? null
  // The proxy fail-open envelope (`{ issue: null, error: ... }`) carries
  // `rate_limited` / `github_unreachable`. A network/HTTP error throws
  // an `ApiError`; we render those as a not-found / error state below
  // rather than degraded mode.
  const proxyError = liveQuery.data?.error ?? null
  const liveErrored = liveQuery.error instanceof ApiError ? liveQuery.error : null
  const liveNotFound = liveErrored?.status === 404

  const inboxBackTo = `/workspaces/${workspace.slug}/issues${
    projectId ? `?project=${encodeURIComponent(projectId)}` : ""
  }`

  // Mobile chrome — title is `#N` so it stays short; the full issue
  // title renders below in the page body. The kebab reuses the same
  // replay/external-link actions as the inbox row. `Replay trigger`
  // needs the issue's *current* labels, so we only enable it (by
  // passing the number) when live data is present — degraded modes
  // (proxy fail-open or live HTTP error) get the disabled state.
  const replayIssueNumber = issue && numberValid ? issueNumber : null
  const headerActions = useMemo(
    () => (
      <IssueActionsMenu
        projectId={projectId || null}
        issueNumber={replayIssueNumber}
        htmlUrl={issue?.html_url ?? null}
        listQueryKey={["project-issue", projectId, numberParam]}
      />
    ),
    [projectId, replayIssueNumber, numberParam, issue?.html_url],
  )
  usePageHeader({
    title: numberValid ? `#${issueNumber}` : "Issue",
    backTo: inboxBackTo,
    actions: headerActions,
  })

  if (!projectId || !numberValid) {
    return (
      <div className="space-y-4">
        <p className="text-destructive">Invalid issue URL.</p>
      </div>
    )
  }

  if (liveQuery.isLoading && skeletonsQuery.isLoading) {
    return (
      <div className="flex min-h-[200px] items-center justify-center">
        <p className="text-muted-foreground">Loading…</p>
      </div>
    )
  }

  // 404 from the backend means the issue genuinely doesn't exist —
  // surface that explicitly even if a stale skeleton happens to match.
  // Other live errors fall through to the error banner below; the
  // skeleton (if present) still anchors the page.
  if (liveNotFound) {
    return (
      <div className="space-y-4">
        <p className="text-destructive">Issue not found.</p>
      </div>
    )
  }

  if (!issue && !skeleton && !liveQuery.isLoading) {
    return (
      <div className="space-y-4">
        <p className="text-destructive">Issue not found.</p>
      </div>
    )
  }

  const display = describe(issue, skeleton, issueNumber)

  return (
    <div className="space-y-4 md:space-y-6">
      {/* Desktop header — back link. Mobile uses the global top bar. */}
      <Link
        to={inboxBackTo}
        className="hidden items-center gap-1 text-sm text-muted-foreground hover:text-foreground md:inline-flex"
      >
        <ArrowLeft className="h-4 w-4" /> Back to Issues
      </Link>

      <div className="flex items-start justify-between gap-4">
        <div className="min-w-0">
          <h1 className="hidden truncate text-2xl font-bold tracking-tight md:block">
            {display.title}
          </h1>
          <p className="text-sm text-muted-foreground font-mono">
            #{issueNumber}
          </p>
        </div>
        <Badge
          variant={display.openState ? "default" : "secondary"}
          className="text-sm"
        >
          {display.stateLabel}
        </Badge>
      </div>

      {proxyError ? (
        <Card>
          <CardContent className="py-3 text-sm text-muted-foreground">
            {proxyError === "rate_limited"
              ? "GitHub rate limit reached. Showing the last-known artifact; details will return in about a minute."
              : "Couldn't reach GitHub. Showing the last-known artifact; details will refresh once the connection recovers."}
          </CardContent>
        </Card>
      ) : null}

      {liveErrored && !liveNotFound ? (
        <Card>
          <CardContent className="py-3 text-sm text-destructive">
            Couldn&apos;t load this issue: {liveErrored.message}
          </CardContent>
        </Card>
      ) : null}

      {/* Desktop: external link as inline button. Mobile uses the
          overflow menu in the global top bar. */}
      {issue?.html_url ? (
        <div className="hidden flex-wrap gap-2 md:flex">
          <Button
            variant="outline"
            size="sm"
            render={
              <a
                href={issue.html_url}
                target="_blank"
                rel="noopener noreferrer"
              />
            }
          >
            <ExternalLink className="mr-1 h-3.5 w-3.5" />
            Open in GitHub
          </Button>
        </div>
      ) : null}

      <div className="grid grid-cols-2 gap-3 md:grid-cols-4">
        <MetaCard label="Author" value={display.author} />
        <MetaCard label="Comments" value={display.commentsLabel} />
        <MetaCard label="Updated" value={display.updatedLabel} />
        <MetaCard
          label="Lifecycle"
          value={skeleton ? skeleton.state.replaceAll("_", " ") : "—"}
        />
      </div>

      {display.labels.length > 0 ? (
        <Card>
          <CardHeader className="px-4 md:px-6">
            <CardTitle className="text-base md:text-lg">Labels</CardTitle>
          </CardHeader>
          <CardContent className="flex flex-wrap gap-2 px-4 md:px-6">
            {display.labels.map((l) => (
              <Badge key={l} variant="outline">
                {l}
              </Badge>
            ))}
          </CardContent>
        </Card>
      ) : null}

      {display.assignees.length > 0 ? (
        <Card>
          <CardHeader className="px-4 md:px-6">
            <CardTitle className="text-base md:text-lg">Assignees</CardTitle>
          </CardHeader>
          <CardContent className="flex flex-wrap gap-2 px-4 md:px-6">
            {display.assignees.map((a) => (
              <Badge key={a} variant="secondary">
                {a}
              </Badge>
            ))}
          </CardContent>
        </Card>
      ) : null}

      <Card>
        <CardHeader className="px-4 md:px-6">
          <CardTitle className="text-base md:text-lg">Description</CardTitle>
        </CardHeader>
        <CardContent className="px-4 md:px-6">
          {issue?.body && issue.body.trim().length > 0 ? (
            // No markdown lib in deps yet — render as preformatted text so
            // line breaks survive. A follow-up will add proper rendering.
            <pre className="whitespace-pre-wrap break-words font-sans text-sm leading-relaxed text-foreground">
              {issue.body}
            </pre>
          ) : (
            <p className="text-sm text-muted-foreground">
              {proxyError
                ? "Description unavailable while GitHub is degraded."
                : "No description."}
            </p>
          )}
        </CardContent>
      </Card>

      <Card>
        <CardHeader className="px-4 md:px-6">
          <CardTitle className="text-base md:text-lg">
            Onsager metadata
          </CardTitle>
        </CardHeader>
        <CardContent className="grid grid-cols-1 gap-3 px-4 md:grid-cols-3 md:px-6">
          <MetaCard
            label="Artifact ID"
            value={skeleton?.id ?? "—"}
            mono
          />
          <MetaCard
            label="Version"
            value={skeleton ? `v${skeleton.current_version}` : "—"}
            mono
          />
          <MetaCard
            label="Last observed"
            value={
              skeleton?.last_observed_at
                ? new Date(skeleton.last_observed_at).toLocaleString()
                : "—"
            }
          />
        </CardContent>
      </Card>
    </div>
  )
}

function MetaCard({
  label,
  value,
  mono,
}: {
  label: string
  value: string
  mono?: boolean
}) {
  return (
    <Card>
      <CardContent className="px-3 py-3">
        <div className="text-xs text-muted-foreground">{label}</div>
        <div
          className={`truncate font-medium ${mono ? "font-mono text-sm" : ""}`}
          title={value}
        >
          {value}
        </div>
      </CardContent>
    </Card>
  )
}

interface IssueDisplay {
  title: string
  openState: boolean
  stateLabel: string
  author: string
  commentsLabel: string
  updatedLabel: string
  labels: string[]
  assignees: string[]
}

function describe(
  issue: ProjectIssueDetail | null,
  skeleton: SpineArtifact | null,
  number: number,
): IssueDisplay {
  if (issue) {
    return {
      title: issue.title,
      openState: issue.state === "open",
      stateLabel: issue.state,
      author: issue.author ?? "—",
      commentsLabel: String(issue.comments),
      updatedLabel: new Date(issue.updated_at).toLocaleString(),
      labels: issue.labels,
      assignees: issue.assignees,
    }
  }
  // Skeleton-only fallback — same shape the inbox uses on a degraded
  // proxy. Lifecycle `draft` ↔ open, `archived` ↔ closed. The title
  // falls back to `Issue #N` rather than the artifact id (which is an
  // internal identifier already shown under "Onsager metadata" and is
  // not user-meaningful).
  const open = skeleton?.state === "draft"
  return {
    title: `Issue #${number}`,
    openState: open,
    stateLabel: open ? "open" : "closed",
    author: "—",
    commentsLabel: "—",
    updatedLabel: skeleton?.last_observed_at
      ? `last seen ${new Date(skeleton.last_observed_at).toLocaleString()}`
      : "—",
    labels: [],
    assignees: [],
  }
}
