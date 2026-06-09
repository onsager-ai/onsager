import { useState } from "react"
import { useMutation, useQueryClient } from "@tanstack/react-query"
import { useNavigate } from "react-router-dom"
import { api, type CreateWorkflowRequest, type GitHubAppInstallation } from "@/lib/api"
import { useOptionalActiveWorkspace } from "@/lib/workspace"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Switch } from "@/components/ui/switch"
import { FlowEditor } from "./FlowEditor"
import { TriggerEditor } from "./TriggerEditor"
import { PresetPicker } from "./PresetPicker"
import {
  documentToCreateRequest,
  emptyDocument,
  isTriggerReady,
  type WorkflowDocument,
} from "./workflow-draft"

export interface WorkflowBuilderProps {
  workspaceId: string
  installations: GitHubAppInstallation[]
  initialDraft?: WorkflowDocument
  onCreated?: (id: string) => void
}

export function WorkflowBuilder({
  workspaceId,
  installations,
  initialDraft,
  onCreated,
}: WorkflowBuilderProps) {
  const [draft, setDraft] = useState<WorkflowDocument>(initialDraft ?? emptyDocument())
  // Creation and activation are distinct acts; default OFF preserves today's
  // draft-by-default behavior (the old "Save as draft" path).
  const [active, setActive] = useState(false)
  const queryClient = useQueryClient()
  const navigate = useNavigate()
  const activeWorkspace = useOptionalActiveWorkspace()

  const canSave =
    draft.name.trim() !== "" &&
    isTriggerReady(draft.trigger) &&
    draft.stages.length > 0

  const create = useMutation({
    mutationFn: (body: CreateWorkflowRequest) => api.createWorkflow(body),
    onSuccess: ({ workflow }) => {
      queryClient.invalidateQueries({ queryKey: ["workflows"] })
      if (onCreated) onCreated(workflow.id)
      else if (activeWorkspace) {
        navigate(`/workspaces/${activeWorkspace.slug}/workflows/${workflow.id}`)
      } else {
        navigate(`/workflows/${workflow.id}`)
      }
    },
  })

  const save = (active: boolean) => {
    if (!canSave) return
    create.mutate(documentToCreateRequest(draft, installations, workspaceId, active))
  }

  return (
    <div className="space-y-4">
      <div className="space-y-1.5">
        <label htmlFor="workflow-name" className="text-sm font-medium">
          Workflow name
        </label>
        <Input
          id="workflow-name"
          value={draft.name}
          onChange={(e) => setDraft({ ...draft, name: e.target.value })}
          placeholder="e.g. Issue → PR pipeline"
        />
      </div>

      <PresetPicker draft={draft} onApply={setDraft} />

      {/* The trigger is how the workflow is invoked — a different kind of
          object than its stages — so it gets its own always-visible section
          rather than living as a node in the stage rail (#572). */}
      <div className="rounded-lg border p-4">
        <TriggerEditor
          workspaceId={workspaceId}
          installations={installations}
          value={draft.trigger}
          onChange={(trigger) => setDraft({ ...draft, trigger })}
        />
      </div>

      <div className="space-y-1.5">
        <span className="text-sm font-medium">Steps</span>
        <FlowEditor draft={draft} onChange={setDraft} />
      </div>

      {create.isError && (
        <p className="text-sm text-destructive">
          {create.error instanceof Error ? create.error.message : "Failed to save"}
        </p>
      )}

      <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
        <label
          htmlFor="workflow-active"
          className="flex items-center gap-2 text-sm font-medium"
        >
          <Switch
            id="workflow-active"
            checked={active}
            onCheckedChange={setActive}
          />
          Active
        </label>
        <Button
          type="button"
          size="lg"
          className="w-full sm:w-auto"
          disabled={!canSave || create.isPending}
          onClick={() => save(active)}
        >
          {create.isPending ? "Creating…" : "Create workflow"}
        </Button>
      </div>
    </div>
  )
}
