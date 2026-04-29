import { useState } from "react"
import { useMutation, useQueryClient } from "@tanstack/react-query"
import {
  ExternalLink,
  MoreHorizontal,
  PlayCircle,
  RefreshCw,
} from "lucide-react"
import { api, type ReplayMatch } from "@/lib/api"
import { Button } from "@/components/ui/button"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"

// Per-row debug actions for the Issues page (spec #203). Workflow
// triggers in production are passive (real GitHub `issues.labeled`
// webhook → `workflow.trigger_fired`); this menu adds the active
// counterpart so a developer can manually drive a single issue
// through the same path while debugging.
//
// Three actions:
// - Refresh: client-side React Query invalidation only.
// - Replay trigger: opens a confirm dialog that previews the
//   matched workflows (server-side dry run) and, on confirm, fires
//   one `workflow.trigger_fired` per match with `source=manual_replay`.
// - Open in GitHub: external link, kept here so the row stays clean.
//
// Skeleton-only rows (no live data) keep the kebab so Refresh stays
// reachable — Refresh re-runs the list query, which is exactly what's
// needed when a row is missing live data. Replay is disabled instead
// of hidden because it needs the issue's current labels, and the
// "Open in GitHub" entry only renders when an `htmlUrl` is known.
export interface IssueActionsMenuProps {
  projectId: string | null
  issueNumber: number | null
  htmlUrl: string | null
  /// Stable React Query key for the issue list this row belongs to,
  /// used when "Refresh this issue" invalidates the cache.
  listQueryKey: readonly unknown[]
}

export function IssueActionsMenu({
  projectId,
  issueNumber,
  htmlUrl,
  listQueryKey,
}: IssueActionsMenuProps) {
  const queryClient = useQueryClient()
  const [confirming, setConfirming] = useState(false)
  const replayable = projectId != null && issueNumber != null

  const preview = useMutation({
    mutationFn: () =>
      api.replayIssueTrigger(projectId!, issueNumber!, { dry_run: true }),
  })

  const fire = useMutation({
    mutationFn: () =>
      api.replayIssueTrigger(projectId!, issueNumber!, { dry_run: false }),
  })

  function openReplay() {
    if (!replayable) return
    preview.reset()
    fire.reset()
    setConfirming(true)
    preview.mutate()
  }

  function refresh() {
    void queryClient.invalidateQueries({ queryKey: listQueryKey })
  }

  return (
    <>
      <DropdownMenu>
        <DropdownMenuTrigger
          render={
            <Button
              type="button"
              variant="ghost"
              size="icon"
              className="h-8 w-8"
              aria-label="Issue actions"
            />
          }
        >
          <MoreHorizontal className="h-4 w-4" />
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end" className="w-52">
          <DropdownMenuItem onClick={refresh}>
            <RefreshCw className="mr-2 h-4 w-4" />
            Refresh this issue
          </DropdownMenuItem>
          <DropdownMenuItem disabled={!replayable} onClick={openReplay}>
            <PlayCircle className="mr-2 h-4 w-4" />
            Replay trigger…
          </DropdownMenuItem>
          {htmlUrl ? (
            <>
              <DropdownMenuSeparator />
              <DropdownMenuItem
                render={
                  <a
                    href={htmlUrl}
                    target="_blank"
                    rel="noopener noreferrer"
                  />
                }
              >
                <ExternalLink className="mr-2 h-4 w-4" />
                Open in GitHub
              </DropdownMenuItem>
            </>
          ) : null}
        </DropdownMenuContent>
      </DropdownMenu>

      <Dialog open={confirming} onOpenChange={setConfirming}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Replay workflow trigger?</DialogTitle>
            <DialogDescription>
              Re-fires <code>workflow.trigger_fired</code> for issue #
              {issueNumber} as if a matching label had just been applied.
              Marked <code>source=manual_replay</code>; downstream stages
              run as they would for the real webhook.
            </DialogDescription>
          </DialogHeader>

          <ReplayPreview
            isLoading={preview.isPending}
            error={
              preview.error instanceof Error ? preview.error.message : null
            }
            matches={preview.data?.matches ?? null}
            fired={fire.data ?? null}
            fireError={
              fire.error instanceof Error ? fire.error.message : null
            }
          />

          <DialogFooter>
            <Button
              type="button"
              variant="outline"
              onClick={() => setConfirming(false)}
              disabled={fire.isPending}
            >
              {fire.data ? "Close" : "Cancel"}
            </Button>
            {fire.data == null ? (
              <Button
                type="button"
                onClick={() => fire.mutate()}
                disabled={
                  fire.isPending ||
                  preview.isPending ||
                  (preview.data?.matches.length ?? 0) === 0
                }
              >
                {fire.isPending ? "Firing…" : "Fire trigger"}
              </Button>
            ) : null}
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </>
  )
}

interface ReplayPreviewProps {
  isLoading: boolean
  error: string | null
  matches: ReplayMatch[] | null
  fired: { event_ids: number[]; matches: ReplayMatch[] } | null
  fireError: string | null
}

function ReplayPreview({
  isLoading,
  error,
  matches,
  fired,
  fireError,
}: ReplayPreviewProps) {
  if (fired) {
    return (
      <div className="space-y-2 text-sm">
        <p className="font-medium text-foreground">
          Fired {fired.event_ids.length}{" "}
          {fired.event_ids.length === 1 ? "workflow" : "workflows"}.
        </p>
        <MatchesList matches={fired.matches} />
        <p className="text-xs text-muted-foreground">
          Spine event IDs: {fired.event_ids.join(", ") || "—"}
        </p>
      </div>
    )
  }
  if (fireError) {
    return <p className="text-sm text-destructive">{fireError}</p>
  }
  if (isLoading) {
    return (
      <p className="text-sm text-muted-foreground">Looking up matches…</p>
    )
  }
  if (error) {
    return <p className="text-sm text-destructive">{error}</p>
  }
  if (!matches) return null
  if (matches.length === 0) {
    return (
      <p className="text-sm text-muted-foreground">
        No active workflows match this issue&apos;s current labels. Add
        the trigger label on GitHub or activate a workflow with a matching
        label, then try again.
      </p>
    )
  }
  return (
    <div className="space-y-2 text-sm">
      <p>
        Will fire {matches.length}{" "}
        {matches.length === 1 ? "workflow" : "workflows"}:
      </p>
      <MatchesList matches={matches} />
    </div>
  )
}

function MatchesList({ matches }: { matches: ReplayMatch[] }) {
  return (
    <ul className="space-y-1 text-sm">
      {matches.map((m) => (
        <li key={`${m.workflow_id}:${m.label}`} className="flex gap-2">
          <span className="font-medium">{m.workflow_name}</span>
          <span className="text-muted-foreground">
            on label <code>{m.label}</code>
          </span>
        </li>
      ))}
    </ul>
  )
}
