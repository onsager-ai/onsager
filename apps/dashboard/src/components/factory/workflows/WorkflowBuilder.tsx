import { useState } from "react"
import { useMutation, useQueryClient } from "@tanstack/react-query"
import { useNavigate } from "react-router-dom"
import { api, type CreateWorkflowRequest, type GitHubAppInstallation } from "@/lib/api"
import { useOptionalActiveWorkspace } from "@/lib/workspace"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { CardStackEditor } from "./CardStackEditor"
import { PresetPicker } from "./PresetPicker"
import {
  draftToCreateRequest,
  emptyDraft,
  isTriggerReady,
  type WorkflowDraft,
} from "./workflow-draft"

export interface WorkflowBuilderProps {
  workspaceId: string
  installations: GitHubAppInstallation[]
  initialDraft?: WorkflowDraft
  onCreated?: (id: string) => void
}

export function WorkflowBuilder({
  workspaceId,
  installations,
  initialDraft,
  onCreated,
}: WorkflowBuilderProps) {
  const [draft, setDraft] = useState<WorkflowDraft>(initialDraft ?? emptyDraft())
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
    create.mutate(draftToCreateRequest(draft, installations, workspaceId, active))
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

      <CardStackEditor
        workspaceId={workspaceId}
        installations={installations}
        draft={draft}
        onChange={setDraft}
      />

      {create.isError && (
        <p className="text-sm text-destructive">
          {create.error instanceof Error ? create.error.message : "Failed to save"}
        </p>
      )}

      <div className="flex flex-col gap-2 sm:flex-row">
        <Button
          type="button"
          variant="outline"
          size="lg"
          className="w-full sm:flex-1"
          disabled={!canSave || create.isPending}
          onClick={() => save(false)}
        >
          Save as draft
        </Button>
        <Button
          type="button"
          size="lg"
          className="w-full sm:flex-1"
          disabled={!canSave || create.isPending}
          onClick={() => save(true)}
        >
          {create.isPending ? "Saving…" : "Activate workflow"}
        </Button>
      </div>
    </div>
  )
}
