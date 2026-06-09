import { useState } from "react"
import { ChevronLeft, ListChecks } from "lucide-react"
import { useIsMobile } from "@/hooks/use-mobile"
import { Button } from "@/components/ui/button"
import type { WorkflowGateKind, WorkflowStage } from "@/lib/api"
import { FlowRail } from "./FlowRail"
import { StageEditor } from "./StageEditor"
import { makeStage, type WorkflowDocument } from "./workflow-draft"

export interface FlowEditorProps {
  draft: WorkflowDocument
  onChange: (next: WorkflowDocument) => void
}

/**
 * Master-detail editor for a workflow's ordered stages. The left
 * {@link FlowRail} shows the whole pipeline and stays visible while the right
 * pane edits the selected stage. The trigger is no longer part of this
 * surface — it's a different kind of object (how the workflow is invoked, not
 * a step) and lives in its own always-visible section above the builder
 * (#572). On mobile the two panes collapse to a list→detail push.
 */
export function FlowEditor({ draft, onChange }: FlowEditorProps) {
  const isMobile = useIsMobile()
  const [selectedId, setSelectedId] = useState<string | null>(
    () => draft.stages[0]?.id ?? null,
  )
  const [mobileDetail, setMobileDetail] = useState(false)

  // Guard a stale selection (e.g. after a preset swap replaces every stage):
  // fall back to the first stage, else nothing.
  const selectedStage = draft.stages.find((s) => s.id === selectedId)
  const effectiveId = selectedStage ? selectedId : (draft.stages[0]?.id ?? null)

  const select = (stageId: string) => {
    setSelectedId(stageId)
    setMobileDetail(true)
  }

  const addStage = (gate: WorkflowGateKind) => {
    const stage = makeStage(gate)
    onChange({ ...draft, stages: [...draft.stages, stage] })
    select(stage.id)
  }

  const updateStage = (id: string, next: WorkflowStage) => {
    onChange({
      ...draft,
      stages: draft.stages.map((s) => (s.id === id ? next : s)),
    })
  }

  const removeStage = (id: string) => {
    const idx = draft.stages.findIndex((s) => s.id === id)
    const stages = draft.stages.filter((s) => s.id !== id)
    onChange({ ...draft, stages })
    if (selectedId === id) {
      const neighbour = stages[idx] ?? stages[idx - 1]
      setSelectedId(neighbour?.id ?? null)
      setMobileDetail(false)
    }
  }

  const index = draft.stages.findIndex((s) => s.id === effectiveId)
  const stage = index >= 0 ? draft.stages[index] : undefined

  const editor = stage ? (
    <StageEditor
      stage={stage}
      index={index}
      onChange={(next) => updateStage(stage.id, next)}
      onRemove={() => removeStage(stage.id)}
    />
  ) : (
    <EmptyDetail />
  )

  const rail = (
    <FlowRail
      draft={draft}
      selectedStageId={effectiveId}
      onSelect={select}
      onAddStage={addStage}
    />
  )

  if (isMobile) {
    return (
      <div className="rounded-lg border p-2">
        {mobileDetail && stage ? (
          <div className="space-y-3 p-2">
            <Button
              type="button"
              variant="ghost"
              size="sm"
              className="-ml-2 text-muted-foreground"
              onClick={() => setMobileDetail(false)}
            >
              <ChevronLeft className="h-4 w-4" />
              All steps
            </Button>
            {editor}
          </div>
        ) : (
          rail
        )}
      </div>
    )
  }

  return (
    <div className="flex min-h-[18rem] overflow-hidden rounded-lg border">
      <div className="w-64 shrink-0 overflow-y-auto border-r bg-muted/20 p-2">
        {rail}
      </div>
      <div className="min-w-0 flex-1 overflow-y-auto p-4">{editor}</div>
    </div>
  )
}

function EmptyDetail() {
  return (
    <div className="flex h-full flex-col items-center justify-center gap-2 py-8 text-center text-muted-foreground">
      <ListChecks className="h-6 w-6" />
      <p className="text-sm">Add a step to start building the pipeline.</p>
    </div>
  )
}
