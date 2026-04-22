import { ArrowDown, Plus } from "lucide-react"
import { Button } from "@/components/ui/button"
import type { GitHubAppInstallation, WorkflowStage } from "@/lib/api"
import { ArtifactFlowOverview } from "./ArtifactFlowOverview"
import {
  DeliverablePanel,
  type DeliverableEntry,
} from "./DeliverablePanel"
import { StageCard } from "./StageCard"
import { TriggerCard } from "./TriggerCard"
import { makeStage, type WorkflowDraft } from "./workflow-draft"

export interface CardStackEditorProps {
  tenantId: string
  installations: GitHubAppInstallation[]
  draft: WorkflowDraft
  onChange: (next: WorkflowDraft) => void
  /// When set (run-detail view), the flow strip highlights this stage and
  /// the deliverable panel becomes visible. Editor view leaves both undefined.
  currentStageIndex?: number
  deliverable?: DeliverableEntry[]
}

export function CardStackEditor({
  tenantId,
  installations,
  draft,
  onChange,
  currentStageIndex,
  deliverable,
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
      {draft.stages.length > 0 && (
        <div className="rounded-md border bg-muted/30 px-3 py-2">
          <div className="mb-1 text-[10px] uppercase tracking-wide text-muted-foreground">
            Flow
          </div>
          <ArtifactFlowOverview
            triggerLabel={draft.trigger.label}
            stages={draft.stages}
            currentStageIndex={currentStageIndex}
          />
        </div>
      )}
      {deliverable && (
        <div className="rounded-md border bg-background px-3 py-2">
          <div className="mb-1 text-[10px] uppercase tracking-wide text-muted-foreground">
            Deliverable
          </div>
          <DeliverablePanel entries={deliverable} />
        </div>
      )}
      <TriggerCard
        tenantId={tenantId}
        installations={installations}
        value={draft.trigger}
        onChange={(trigger) => onChange({ ...draft, trigger })}
      />
      {draft.stages.map((s, i) => (
        <div key={s.id} className="space-y-1">
          <div className="flex justify-center">
            <ArrowDown
              aria-hidden
              className="h-4 w-4 text-muted-foreground/70"
            />
          </div>
          <StageCard
            stage={s}
            index={i}
            onChange={(next) => updateStage(i, next)}
            onRemove={() => removeStage(i)}
          />
        </div>
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
