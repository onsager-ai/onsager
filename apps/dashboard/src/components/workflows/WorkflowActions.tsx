import { useState } from "react"
import { useMutation, useQueryClient } from "@tanstack/react-query"
import { useNavigate } from "react-router-dom"
import { MoreHorizontal, Pause, Play, Rocket, Trash2 } from "lucide-react"
import { api, type Workflow } from "@/lib/api"
import {
  HitlActionButton,
  HitlActionDialog,
} from "@/components/chat/HitlActionButton"
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
// The "Run a batch" affordance (ADR 0024 / 0025, spec #514) gates on the
// `readiness` axis, not `active`: a `ready + off` (manual-only) workflow
// is a first-class state that still earns the button. The button only
// applies to manual-trigger workflows — `run_workflow` requires a
// `TriggerKind::Manual` binding — so a non-manual `ready` workflow shows
// no manual-run button (it fires on its own trigger). The mutation
// routes through a `HitlCard` (HITL principle 1), reusing the shared
// `run_workflow` MCP path rather than the REST manual-trigger endpoint.
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
  const [runOpen, setRunOpen] = useState(false)

  const isActive = workflow.status === "active"
  const isManualTrigger = workflow.trigger.kind_tag === "manual"
  const manualName = workflow.trigger.manual_name ?? ""
  // ADR 0024: `ready` (any trigger binding), not `active`, earns the run
  // button. `run_workflow` needs a `Manual` trigger, so the affordance is
  // manual-trigger-only.
  const canRunBatch =
    workflow.readiness === "ready" && isManualTrigger && manualName !== ""

  const onRunCommitted = () => {
    // The trigger.fired event lands within ~50ms; refresh runs so the
    // new artifact appears without waiting for the 5s poll.
    queryClient.invalidateQueries({ queryKey: ["workflow-runs", workflow.id] })
    queryClient.invalidateQueries({ queryKey: ["spine-events"] })
  }

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
          {canRunBatch && (
            <DropdownMenuItem onClick={() => setRunOpen(true)}>
              <Rocket className="mr-2 h-4 w-4" />
              Run a batch
            </DropdownMenuItem>
          )}
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
        {canRunBatch && (
          <HitlActionButton
            toolName="run_workflow"
            args={{ workflow_id: workflow.id, trigger_name: manualName }}
            onCommitted={onRunCommitted}
            variant="default"
            size="sm"
          >
            <Rocket className="h-4 w-4" />
            Run a batch
          </HitlActionButton>
        )}
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
      {/* Menu-variant run trigger opens this controlled review dialog;
          the buttons variant uses its own HitlActionButton inline. */}
      {variant === "menu" && canRunBatch && (
        <HitlActionDialog
          open={runOpen}
          onOpenChange={setRunOpen}
          toolName="run_workflow"
          args={{ workflow_id: workflow.id, trigger_name: manualName }}
          onCommitted={onRunCommitted}
        />
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
    </>
  )
}
