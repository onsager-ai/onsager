import { useState } from "react"
import { useMutation, useQueryClient } from "@tanstack/react-query"
import { useNavigate } from "react-router-dom"
import { MoreHorizontal, Pause, Play, Trash2 } from "lucide-react"
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
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"

// Lifecycle controls for a single workflow. Lives on the detail page so
// pause/resume and destructive delete are one tap away from "what does
// this workflow do?" — the same place the user just evaluated it.
//
// `status` maps to the backend's `active` flag (draft/paused both
// render as "Publish", active renders as "Pause"). Delete always asks
// for confirmation in a modal — a workflow is durable state with a
// webhook and a label side effect, not a typo.
//
// `variant`:
//   - "buttons" (default): full-width labeled buttons. Used in the
//     desktop page-level header block.
//   - "menu": single `⋯` icon that opens a DropdownMenu. Used in the
//     mobile chrome bar where space is scarce — the dashboard-ui rule
//     is "always overflow when actions > 1" so the bar stays
//     predictable.
export interface WorkflowActionsProps {
  workflow: Workflow
  variant?: "buttons" | "menu"
}

export function WorkflowActions({
  workflow,
  variant = "buttons",
}: WorkflowActionsProps) {
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

  const controls =
    variant === "menu" ? (
      <DropdownMenu>
        <DropdownMenuTrigger
          render={
            <Button
              type="button"
              variant="ghost"
              size="icon"
              className="h-9 w-9"
              aria-label="Workflow actions"
            />
          }
        >
          <MoreHorizontal className="h-5 w-5" />
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end" className="w-44">
          <DropdownMenuItem
            disabled={toggle.isPending}
            onClick={() => toggle.mutate(!isActive)}
          >
            <ToggleIcon className="mr-2 h-4 w-4" />
            {toggleLabel}
          </DropdownMenuItem>
          <DropdownMenuItem
            variant="destructive"
            onClick={() => setConfirming(true)}
          >
            <Trash2 className="mr-2 h-4 w-4" />
            Delete
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
    ) : (
      <div
        className="flex flex-wrap items-center gap-2"
        data-testid="workflow-actions"
      >
        <Button
          type="button"
          variant={isActive ? "outline" : "default"}
          size="sm"
          disabled={toggle.isPending}
          onClick={() => toggle.mutate(!isActive)}
        >
          <ToggleIcon className="h-4 w-4" />
          {toggleLabel}
        </Button>
        <Button
          type="button"
          variant="outline"
          size="sm"
          onClick={() => setConfirming(true)}
        >
          <Trash2 className="h-4 w-4" />
          Delete
        </Button>
        {toggle.isError && (
          <p className="w-full text-xs text-destructive">
            {toggle.error instanceof Error
              ? toggle.error.message
              : "Failed to update workflow"}
          </p>
        )}
      </div>
    )

  return (
    <>
      {controls}
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
    </>
  )
}
