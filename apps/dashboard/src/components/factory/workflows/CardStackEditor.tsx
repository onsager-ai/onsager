import { Plus } from "lucide-react"
import { Button } from "@/components/ui/button"
import type { GitHubAppInstallation, WorkflowStage } from "@/lib/api"
import { StageCard } from "./StageCard"
import { TriggerCard } from "./TriggerCard"
import { makeStage, type WorkflowDraft } from "./workflow-draft"

export interface CardStackEditorProps {
  tenantId: string
  installations: GitHubAppInstallation[]
  draft: WorkflowDraft
  onChange: (next: WorkflowDraft) => void
}

export function CardStackEditor({
  tenantId,
  installations,
  draft,
  onChange,
}: CardStackEditorProps) {
  const updateStage = (idx: number, next: WorkflowStage) => {
    const stages = draft.stages.slice()
    stages[idx] = next
    onChange({ ...draft, stages })
  }
  const removeStage = (idx: number) => {
    const stages = draft.stages.slice()
    stages.splice(idx, 1)
    onChange({ ...draft, stages })
  }
  const addStage = () => {
    onChange({
      ...draft,
      stages: [...draft.stages, makeStage("agent-session")],
    })
  }

  return (
    <div className="space-y-3">
      <TriggerCard
        tenantId={tenantId}
        installations={installations}
        value={draft.trigger}
        onChange={(trigger) => onChange({ ...draft, trigger })}
      />
      {draft.stages.map((s, i) => (
        <StageCard
          key={s.id}
          stage={s}
          index={i}
          onChange={(next) => updateStage(i, next)}
          onRemove={() => removeStage(i)}
        />
      ))}
      <Button
        type="button"
        variant="outline"
        className="w-full justify-center gap-2"
        onClick={addStage}
      >
        <Plus className="h-4 w-4" />
        Add stage
      </Button>
    </div>
  )
}
