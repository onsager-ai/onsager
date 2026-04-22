import { useState } from "react"
import { useMutation, useQueryClient } from "@tanstack/react-query"
import { useNavigate } from "react-router-dom"
import { api, type CreateWorkflowRequest, type GitHubAppInstallation } from "@/lib/api"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { CardStackEditor } from "./CardStackEditor"
import { ChatBuilder } from "./ChatBuilder"
import { PresetPicker } from "./PresetPicker"
import {
  draftToRequestTrigger,
  emptyDraft,
  isTriggerReady,
  type WorkflowDraft,
} from "./workflow-draft"

export interface WorkflowBuilderProps {
  tenantId: string
  installations: GitHubAppInstallation[]
  initialDraft?: WorkflowDraft
  onCreated?: (id: string) => void
}

export function WorkflowBuilder({
  tenantId,
  installations,
  initialDraft,
  onCreated,
}: WorkflowBuilderProps) {
  const [draft, setDraft] = useState<WorkflowDraft>(initialDraft ?? emptyDraft())
  const queryClient = useQueryClient()
  const navigate = useNavigate()

  const canSave =
    draft.name.trim() !== "" &&
    isTriggerReady(draft.trigger) &&
    draft.stages.length > 0

  const create = useMutation({
    mutationFn: (body: CreateWorkflowRequest) => api.createWorkflow(body),
    onSuccess: ({ workflow }) => {
      queryClient.invalidateQueries({ queryKey: ["workflows"] })
      if (onCreated) onCreated(workflow.id)
      else navigate(`/workflows/${workflow.id}`)
    },
  })

  const save = () => {
    if (!canSave) return
    create.mutate({
      tenant_id: tenantId,
      name: draft.name.trim(),
      trigger: draftToRequestTrigger(draft.trigger),
      stages: draft.stages,
      activate: true,
    })
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

      <ChatBuilder draft={draft} onChange={setDraft} />

      <CardStackEditor
        tenantId={tenantId}
        installations={installations}
        draft={draft}
        onChange={setDraft}
      />

      {create.isError && (
        <p className="text-sm text-destructive">
          {create.error instanceof Error ? create.error.message : "Failed to save"}
        </p>
      )}

      <Button
        type="button"
        size="lg"
        className="w-full"
        disabled={!canSave || create.isPending}
        onClick={save}
      >
        {create.isPending ? "Saving…" : "Activate workflow"}
      </Button>
    </div>
  )
}
