import { useMemo } from "react"
import { useQuery } from "@tanstack/react-query"
import { api, type ProjectIssueDetail } from "@/lib/api"
import type { ArtifactDetail } from "@/lib/api/generated/ArtifactDetail"
import { Badge } from "@/components/ui/badge"
import { Card, CardContent } from "@/components/ui/card"
import { IssueActionsMenu } from "@/components/IssueActionsMenu"

// Content section for ArtifactDetail (ADR 0021 inv. 1 + #491). Dispatches
// on `artifact.kind`; per-kind branches land incrementally. Three ship
// first: `github_issue` (absorbs the retired IssueDetailPage render per
// #492), `markdown`, and the default raw fallback.

export function ArtifactContent({ artifact }: { artifact: ArtifactDetail }) {
  switch (artifact.kind) {
    case "github_issue":
      return <GithubIssueContent artifact={artifact} />
    case "markdown":
      return <MarkdownContent artifact={artifact} />
    default:
      return <RawContent artifact={artifact} />
  }
}

function latestVersion(artifact: ArtifactDetail) {
  if (!artifact.versions || artifact.versions.length === 0) return null
  return [...artifact.versions].sort((a, b) => b.version - a.version)[0]
}

// Parse the reference-only external handle a `github_issue` artifact
// carries: `github:project:<projectId>:issue:<number>` (#170).
function parseIssueRef(
  externalRef: string | null,
): { projectId: string; number: number } | null {
  if (!externalRef) return null
  const m = /^github:project:(.+):issue:(\d+)$/.exec(externalRef)
  if (!m) return null
  return { projectId: m[1], number: Number.parseInt(m[2], 10) }
}

function GithubIssueContent({ artifact }: { artifact: ArtifactDetail }) {
  const ref = useMemo(
    () => parseIssueRef(artifact.external_ref),
    [artifact.external_ref],
  )

  // Live-hydrate the provider-authored fields (title/body/labels/etc.)
  // through the same proxy IssueDetailPage used (#170 fail-open): the
  // envelope carries `{ issue: null, error }` when GitHub is rate-limited
  // or unreachable, surfaced as the warning below.
  const { data, isLoading, isError, error } = useQuery({
    queryKey: ["project-issue", ref?.projectId, ref?.number],
    queryFn: () => api.getProjectIssue(ref!.projectId, ref!.number),
    enabled: !!ref,
    refetchInterval: 60_000,
  })

  if (!ref) {
    return (
      <p className="text-sm text-muted-foreground">
        This issue artifact has no resolvable GitHub reference.
      </p>
    )
  }

  const issue: ProjectIssueDetail | null = data?.issue ?? null
  // Two failure shapes: the proxy fail-open envelope (`{ issue: null,
  // error }`, rate-limited / unreachable) and a thrown ApiError (404,
  // network). The retired IssueDetailPage surfaced both; without this
  // the github_issue branch would render blank "No description" on a
  // hard error rather than a degraded message.
  const proxyError = data?.error ?? null
  const liveError =
    isError && error instanceof Error ? error.message : null

  return (
    <div className="space-y-4">
      <div className="flex items-start justify-between gap-3">
        {issue ? (
          <h3 className="text-base font-semibold leading-snug">{issue.title}</h3>
        ) : (
          <span />
        )}
        {/* Issue-level actions (Replay trigger / Open in GitHub) follow
            the artifact onto its detail page now that IssueDetailPage is
            retired (#492). Replay needs live labels, so it's enabled only
            when the live issue resolved. */}
        <IssueActionsMenu
          projectId={ref.projectId}
          issueNumber={issue ? ref.number : null}
          htmlUrl={issue?.html_url ?? null}
          listQueryKey={["project-issue", ref.projectId, ref.number]}
        />
      </div>

      {proxyError ? (
        <p className="rounded-md border bg-muted/30 px-3 py-2 text-sm text-muted-foreground">
          {proxyError === "rate_limited"
            ? "GitHub rate limit reached. Showing the last-known artifact; details will return in about a minute."
            : "Couldn't reach GitHub. Showing the last-known artifact; details will refresh once the connection recovers."}
        </p>
      ) : liveError ? (
        <p className="rounded-md border border-destructive/40 bg-destructive/5 px-3 py-2 text-sm text-destructive">
          Couldn&apos;t load the live issue from GitHub: {liveError}
        </p>
      ) : null}

      {/* GitHub-fed single-value fields (former IssueDetail MetaCard grid,
          minus Lifecycle which is an artifact attribute and lives in the
          ArtifactDetail header strip per #492). */}
      <div className="grid grid-cols-3 gap-x-4 gap-y-2 text-sm">
        <IssueMeta label="Author" value={issue?.author ?? "—"} />
        <IssueMeta
          label="Comments"
          value={issue ? String(issue.comments) : "—"}
        />
        <IssueMeta
          label="Updated"
          value={
            issue ? new Date(issue.updated_at).toLocaleString() : "—"
          }
        />
      </div>

      {issue && issue.labels.length > 0 ? (
        <div className="flex flex-wrap gap-2">
          {issue.labels.map((l) => (
            <Badge key={l} variant="outline">
              {l}
            </Badge>
          ))}
        </div>
      ) : null}

      {issue && issue.assignees.length > 0 ? (
        <div className="flex flex-wrap gap-2">
          {issue.assignees.map((a) => (
            <Badge key={a} variant="secondary">
              {a}
            </Badge>
          ))}
        </div>
      ) : null}

      {/* Description — self-bounded body, the one place ADR 0021 inv. 5
          keeps a Card. No markdown lib in deps yet; preformatted text
          preserves line breaks (#491 leaves the lib choice to a
          follow-up). */}
      <Card>
        <CardContent className="px-4 py-4 md:px-6">
          {issue?.body && issue.body.trim().length > 0 ? (
            <pre className="whitespace-pre-wrap break-words font-sans text-sm leading-relaxed text-foreground">
              {issue.body}
            </pre>
          ) : (
            <p className="text-sm text-muted-foreground">
              {isLoading
                ? "Loading description…"
                : proxyError || liveError
                  ? "Description unavailable while GitHub is degraded."
                  : "No description."}
            </p>
          )}
        </CardContent>
      </Card>
    </div>
  )
}

function MarkdownContent({ artifact }: { artifact: ArtifactDetail }) {
  const version = latestVersion(artifact)
  return (
    <div className="space-y-3">
      <Badge variant="outline">{artifact.kind}</Badge>
      <Card>
        <CardContent className="px-4 py-4 md:px-6">
          {version?.change_summary ? (
            <pre className="whitespace-pre-wrap break-words font-sans text-sm leading-relaxed text-foreground">
              {version.change_summary}
            </pre>
          ) : (
            <p className="text-sm text-muted-foreground">No content summary.</p>
          )}
          {version?.content_ref_uri ? (
            <p className="mt-3 truncate font-mono text-xs text-muted-foreground">
              {version.content_ref_uri}
            </p>
          ) : null}
        </CardContent>
      </Card>
      <p className="text-xs text-muted-foreground">
        Inline markdown rendering ships with the per-kind content polish
        follow-up (#491 out of scope).
      </p>
    </div>
  )
}

function RawContent({ artifact }: { artifact: ArtifactDetail }) {
  const version = latestVersion(artifact)
  return (
    <div className="space-y-2 text-sm">
      <Badge variant="outline">{artifact.kind}</Badge>
      {version ? (
        <div className="space-y-1">
          <div className="text-muted-foreground">
            {version.change_summary || "No content summary."}
          </div>
          {version.content_ref_uri ? (
            <div className="truncate font-mono text-xs text-muted-foreground">
              {version.content_ref_uri}
            </div>
          ) : null}
        </div>
      ) : (
        <p className="text-muted-foreground">
          No content yet for this artifact.
        </p>
      )}
    </div>
  )
}

function IssueMeta({ label, value }: { label: string; value: string }) {
  return (
    <div className="min-w-0 space-y-0.5">
      <div className="text-[10px] uppercase tracking-wide text-muted-foreground">
        {label}
      </div>
      <div className="truncate" title={value}>
        {value}
      </div>
    </div>
  )
}
