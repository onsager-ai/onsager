import type { ReactNode } from "react"
import { CircleDot, GitPullRequest } from "lucide-react"
import { Badge } from "@/components/ui/badge"
import type { WorkflowArtifactKind } from "@/lib/api"
import { artifactKindMeta } from "./workflow-meta"

// The "product" view of a workflow run — a live snapshot of the
// Deliverable (issue #100/#101). Sits beside the flow strip so the UI
// answers two different questions: "where are we?" (flow strip) vs.
// "what have we built?" (this panel).
//
// V1 renders a compact card per artifact reference the workflow run
// currently points at. Per-kind rich summaries (commits count, checks by
// source) land as the backend starts emitting the intrinsic fields from
// #103; for now we surface the kind + id, which is already enough to
// fix the duplicate-PR regression from #100.
export interface DeliverableEntry {
  kind: WorkflowArtifactKind
  artifact_id: string
  // Optional per-kind intrinsic payload (issue #103). For a PR this holds
  // `{ target, commits, checks, reviews, merged, closes_issue }`; unknown
  // kinds pass-through as a generic JSON blob.
  intrinsic?: Record<string, unknown>
}

export interface DeliverablePanelProps {
  entries: DeliverableEntry[]
  empty?: ReactNode
}

export function DeliverablePanel({ entries, empty }: DeliverablePanelProps) {
  if (entries.length === 0) {
    return (
      <div className="rounded-md border border-dashed border-muted-foreground/30 bg-muted/20 px-3 py-3 text-xs text-muted-foreground">
        {empty ?? "No artifacts yet — the workflow hasn't produced output."}
      </div>
    )
  }

  return (
    <div
      className="space-y-2"
      data-testid="workflow-deliverable-panel"
    >
      {entries.map((entry) => (
        <DeliverableCard key={`${entry.kind}-${entry.artifact_id}`} entry={entry} />
      ))}
    </div>
  )
}

// Canonical PR kind detection (issue #102). Mirrors the alias map in
// `workflow-meta.ts` so legacy persisted `github-pr` / `PullRequest`
// values still get the rich PR card instead of falling through to the
// generic one.
function isPrDeliverableKind(kind: WorkflowArtifactKind): boolean {
  return kind === "PR" || kind === "github-pr" || kind === "PullRequest"
}

function DeliverableCard({ entry }: { entry: DeliverableEntry }) {
  const meta = artifactKindMeta(entry.kind)
  const Icon = meta.icon

  if (isPrDeliverableKind(entry.kind)) {
    return <PrCard entry={entry} />
  }

  return (
    <div className="rounded-md border bg-card px-3 py-2">
      <div className="flex items-center gap-2 text-sm">
        <Icon aria-hidden className="h-4 w-4 text-muted-foreground" />
        <span className="font-medium">{meta.shortLabel}</span>
        <code className="ml-auto text-xs text-muted-foreground">
          {entry.artifact_id}
        </code>
      </div>
    </div>
  )
}

function PrCard({ entry }: { entry: DeliverableEntry }) {
  const pr = (entry.intrinsic ?? {}) as {
    number?: number
    target?: { repo?: string; branch?: string }
    commits?: Array<{ sha?: string; message?: string }>
    checks?: Record<string, { status?: string; conclusion?: string }>
    reviews?: Record<string, { state?: string }>
    merged?: { sha?: string; merged_at?: string } | null
  }

  const checks = Object.entries(pr.checks ?? {})
  const reviews = Object.entries(pr.reviews ?? {})

  return (
    <div className="rounded-md border bg-card px-3 py-2">
      <div className="flex items-center gap-2">
        <GitPullRequest aria-hidden className="h-4 w-4 text-muted-foreground" />
        <span className="font-medium text-sm">
          PR {pr.number != null ? `#${pr.number}` : ""}
        </span>
        {pr.target?.branch && (
          <span className="text-xs text-muted-foreground">
            → {pr.target.branch}
          </span>
        )}
        <code className="ml-auto text-xs text-muted-foreground">
          {entry.artifact_id}
        </code>
      </div>

      <dl className="mt-2 grid grid-cols-2 gap-x-3 gap-y-1 text-xs text-muted-foreground">
        <dt>Commits</dt>
        <dd className="text-foreground">{pr.commits?.length ?? 0}</dd>

        <dt>Checks</dt>
        <dd className="flex flex-wrap gap-1">
          {checks.length === 0 && <span>—</span>}
          {checks.map(([src, v]) => (
            <Badge
              key={src}
              variant={
                v.conclusion === "success" || v.conclusion === "pass"
                  ? "default"
                  : "secondary"
              }
              className="px-1.5 text-[10px]"
            >
              {src}: {v.conclusion ?? v.status ?? "?"}
            </Badge>
          ))}
        </dd>

        <dt>Reviews</dt>
        <dd className="flex flex-wrap gap-1">
          {reviews.length === 0 && <span>—</span>}
          {reviews.map(([reviewer, v]) => (
            <Badge
              key={reviewer}
              variant={v.state === "approved" ? "default" : "secondary"}
              className="px-1.5 text-[10px]"
            >
              {reviewer}: {v.state ?? "?"}
            </Badge>
          ))}
        </dd>

        <dt>Merged</dt>
        <dd className="text-foreground">
          {pr.merged?.sha ? (
            <span className="flex items-center gap-1">
              <CircleDot aria-hidden className="h-3 w-3 text-green-600" />
              {pr.merged.sha.slice(0, 7)}
            </span>
          ) : (
            "—"
          )}
        </dd>
      </dl>
    </div>
  )
}
