import { useState } from "react"
import { useMutation, useQueryClient } from "@tanstack/react-query"
import { useNavigate } from "react-router-dom"
import { Pause, Play, Trash2 } from "lucide-react"
import { api, type Workflow } from "@/lib/api"
import { Button } from "@/components/ui/button"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"

// Lifecycle controls for a single workflow. Lives on the detail page so
// pause/resume and destructive delete are one tap away from "what does
// this workflow do?" — the same place the user just evaluated it.
//
// `status` maps to the backend's `active` flag (draft/paused both
// render as "Publish", active renders as "Pause"). Delete always asks
// for confirmation in a modal — a workflow is durable state with a
// webhook and a label side effect, not a typo.
export interface WorkflowActionsProps {
  workflow: Workflow
  // Icon-only rendering for the mobile header slot. Falls back to
  // labeled buttons in the page-level desktop block.
  compact?: boolean
}

export function WorkflowActions({ workflow, compact = false }: WorkflowActionsProps) {
  const queryClient = useQueryClient()
  const navigate = useNavigate()
  const [confirming, setConfirming] = useState(false)

  const isActive = workflow.status === "active"

  const toggle = useMutation({
    mutationFn: (active: boolean) => api.setWorkflowActive(workflow.id, active),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["workflow", workflow.id] })
      queryClient.invalidateQueries({ queryKey: ["workflows"] })
    },
  })

  const remove = useMutation({
    mutationFn: () => api.deleteWorkflow(workflow.id),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["workflows"] })
      setConfirming(false)
      navigate("/workflows")
    },
  })

  const toggleLabel = isActive
    ? "Pause"
    : workflow.status === "draft"
      ? "Publish"
      : "Resume"
  const ToggleIcon = isActive ? Pause : Play

  return (
    <div
      className={
        compact
          ? "flex items-center gap-1"
          : "flex flex-wrap items-center gap-2"
      }
      data-testid="workflow-actions"
    >
      <Button
        type="button"
        variant={compact ? "ghost" : isActive ? "outline" : "default"}
        size={compact ? "icon" : "sm"}
        className={compact ? "h-9 w-9" : undefined}
        disabled={toggle.isPending}
        onClick={() => toggle.mutate(!isActive)}
        aria-label={compact ? toggleLabel : undefined}
        title={compact ? toggleLabel : undefined}
      >
        <ToggleIcon className="h-4 w-4" />
        {!compact && toggleLabel}
      </Button>

      <Button
        type="button"
        variant={compact ? "ghost" : "outline"}
        size={compact ? "icon" : "sm"}
        className={compact ? "h-9 w-9" : undefined}
        onClick={() => setConfirming(true)}
        aria-label={compact ? "Delete" : undefined}
        title={compact ? "Delete" : undefined}
      >
        <Trash2 className="h-4 w-4" />
        {!compact && "Delete"}
      </Button>

      {!compact && toggle.isError && (
        <p className="w-full text-xs text-destructive">
          {toggle.error instanceof Error
            ? toggle.error.message
            : "Failed to update workflow"}
        </p>
      )}

      <Dialog open={confirming} onOpenChange={setConfirming}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Delete workflow?</DialogTitle>
            <DialogDescription>
              Removes <span className="font-medium">{workflow.name}</span> and
              its stage chain. The GitHub webhook is dropped if no other
              workflow on {workflow.trigger.repo_owner}/
              {workflow.trigger.repo_name} still needs it. In-flight runs
              aren&apos;t affected retroactively.
            </DialogDescription>
          </DialogHeader>
          {remove.isError && (
            <p className="text-sm text-destructive">
              {remove.error instanceof Error
                ? remove.error.message
                : "Delete failed"}
            </p>
          )}
          <DialogFooter>
            <Button
              type="button"
              variant="outline"
              onClick={() => setConfirming(false)}
              disabled={remove.isPending}
            >
              Cancel
            </Button>
            <Button
              type="button"
              variant="destructive"
              onClick={() => remove.mutate()}
              disabled={remove.isPending}
            >
              {remove.isPending ? "Deleting…" : "Delete workflow"}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  )
}
