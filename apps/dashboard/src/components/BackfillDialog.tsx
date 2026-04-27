import { cloneElement, isValidElement, useState, type ReactElement } from "react"
import { useMutation, useQueryClient } from "@tanstack/react-query"
import { Loader2 } from "lucide-react"

import { api, ApiError, type BackfillRequestBody } from "@/lib/api"
import { Button } from "@/components/ui/button"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { Input } from "@/components/ui/input"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"

const HARD_CAP = 200
const DEFAULT_CAP = 100

/**
 * Bulk-ingest a project's existing GitHub issues + PRs as reference-only
 * skeleton artifacts (specs #167, #170, #171). The dashboard inbox is
 * empty until either:
 *   - new issues/PRs arrive via webhook (live ingestion), or
 *   - a backfill walks the existing GitHub state.
 *
 * Surfaced in two places (#168):
 *   - As an action on `/issues` for projects that haven't been backfilled.
 *   - In the project-onboarding flow on `WorkspaceCard.tsx` after add-project.
 */
export function BackfillDialog({
  projectId,
  repoLabel,
  open: controlledOpen,
  onOpenChange,
  trigger,
  onCompleted,
}: {
  projectId: string
  /** `owner/name` for display only; the request is keyed on `projectId`. */
  repoLabel?: string
  open?: boolean
  onOpenChange?: (open: boolean) => void
  /// Element rendered as the open-trigger when the dialog is uncontrolled.
  /// We `cloneElement` to add an `onClick` rather than wrapping in
  /// DialogTrigger — the existing dialogs in this codebase manage `open`
  /// state externally for consistency with NewWorkspaceDialog.
  trigger?: ReactElement<{ onClick?: (e: React.MouseEvent) => void }>
  onCompleted?: () => void
}) {
  const isControlled = controlledOpen !== undefined
  const [internalOpen, setInternalOpen] = useState(false)
  const open = isControlled ? controlledOpen : internalOpen
  const setOpen = (v: boolean) => {
    if (!isControlled) setInternalOpen(v)
    onOpenChange?.(v)
  }

  const queryClient = useQueryClient()
  const [cap, setCap] = useState(DEFAULT_CAP)
  const [strategy, setStrategy] =
    useState<NonNullable<BackfillRequestBody["strategy"]>>("recent")
  const [stateFilter, setStateFilter] =
    useState<NonNullable<BackfillRequestBody["state"]>>("open")
  const [error, setError] = useState<string | null>(null)

  const mutation = useMutation({
    mutationFn: () =>
      api.backfillProject(projectId, { cap, strategy, state: stateFilter }),
    onSuccess: () => {
      // Bust the artifact list + per-project live caches so the next render
      // pulls the new skeleton rows.
      queryClient.invalidateQueries({ queryKey: ["artifacts"] })
      queryClient.invalidateQueries({ queryKey: ["project-issues", projectId] })
      queryClient.invalidateQueries({ queryKey: ["project-pulls", projectId] })
      onCompleted?.()
      setOpen(false)
    },
    onError: (e) => {
      // Surface the server's error message when available; otherwise a
      // generic fallback. Inputs are validated client-side; a 5xx here
      // typically means the GitHub App isn't configured or rate-limited.
      if (e instanceof ApiError) {
        setError(e.message || "Backfill failed")
      } else {
        setError("Backfill failed")
      }
    },
  })

  const handleSubmit = () => {
    setError(null)
    mutation.mutate()
  }

  const triggerElement =
    trigger && isValidElement(trigger)
      ? cloneElement(trigger, {
          onClick: (e: React.MouseEvent) => {
            trigger.props.onClick?.(e)
            if (!e.defaultPrevented) setOpen(true)
          },
        })
      : null

  return (
    <>
      {triggerElement}
      <Dialog open={open} onOpenChange={setOpen}>
        <DialogContent>
        <DialogHeader>
          <DialogTitle>Backfill from GitHub</DialogTitle>
          <DialogDescription>
            Ingest existing issues and pull requests
            {repoLabel ? ` from ${repoLabel}` : ""} as reference-only artifacts.
            Live updates start automatically; this is for pre-existing items.
          </DialogDescription>
        </DialogHeader>

        <div className="grid gap-4 py-2">
          <div className="grid gap-2">
            <label className="text-sm font-medium" htmlFor="backfill-cap">
              Cap
            </label>
            <Input
              id="backfill-cap"
              type="number"
              min={1}
              max={HARD_CAP}
              value={cap}
              onChange={(e) => {
                const n = parseInt(e.target.value, 10)
                if (Number.isFinite(n)) {
                  setCap(Math.min(HARD_CAP, Math.max(1, n)))
                }
              }}
            />
            <p className="text-xs text-muted-foreground">
              Maximum {HARD_CAP}. Larger backfills require the CLI
              (<code className="font-mono">onsager project sync</code>).
            </p>
          </div>

          <div className="grid gap-2">
            <label className="text-sm font-medium">Strategy</label>
            <Select
              value={strategy}
              onValueChange={(v) =>
                setStrategy(v as NonNullable<BackfillRequestBody["strategy"]>)
              }
            >
              <SelectTrigger>
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="recent">Recent (newest first)</SelectItem>
                <SelectItem value="active">Active (skip stale)</SelectItem>
                <SelectItem value="refract">Refract (priority-ranked)</SelectItem>
              </SelectContent>
            </Select>
          </div>

          <div className="grid gap-2">
            <label className="text-sm font-medium">State</label>
            <Select
              value={stateFilter}
              onValueChange={(v) =>
                setStateFilter(v as NonNullable<BackfillRequestBody["state"]>)
              }
            >
              <SelectTrigger>
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="open">Open only</SelectItem>
                <SelectItem value="closed">Closed only</SelectItem>
                <SelectItem value="all">All states</SelectItem>
              </SelectContent>
            </Select>
          </div>

          {error ? (
            <p className="text-sm text-destructive">{error}</p>
          ) : null}
        </div>

        <DialogFooter>
          <Button
            variant="outline"
            type="button"
            onClick={() => setOpen(false)}
          >
            Cancel
          </Button>
          <Button
            type="button"
            onClick={handleSubmit}
            disabled={mutation.isPending}
          >
            {mutation.isPending ? (
              <>
                <Loader2 className="h-4 w-4 animate-spin" data-icon="inline-start" />
                Backfilling…
              </>
            ) : (
              "Backfill"
            )}
          </Button>
        </DialogFooter>
        </DialogContent>
      </Dialog>
    </>
  )
}
