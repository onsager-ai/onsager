import { useState } from "react"
import { ChevronLeft, ListPlus } from "lucide-react"
import { useIsMobile } from "@/hooks/use-mobile"
import { Button } from "@/components/ui/button"
import type { WorkflowGateKind, WorkflowStage } from "@/lib/api"
import { StageEditor } from "./StageEditor"
import { StepRail } from "./StepRail"
import { makeStage, type WorkflowDocument } from "./workflow-draft"

export interface StepsEditorProps {
  draft: WorkflowDocument
  onChange: (next: WorkflowDocument) => void
}

/**
 * "Steps" tab content: a master-detail editor for the stage list. The left
 * {@link StepRail} shows the whole pipeline and stays visible while the right
 * pane edits the selected stage. On mobile the two panes collapse to a
 * list→detail push (one column, same model). Stages are addressed by id (not
 * index) so the selection survives reorders and neighbour removals.
 */
export function StepsEditor({ draft, onChange }: StepsEditorProps) {
  const isMobile = useIsMobile()
  const [selectedId, setSelectedId] = useState<string | null>(
    () => draft.stages[0]?.id ?? null,
  )
  const [mobileDetail, setMobileDetail] = useState(false)

  // Guard a stale selection (e.g. after the selected stage was removed): fall
  // back to the first stage, else nothing.
  const selected =
    draft.stages.find((s) => s.id === selectedId) ?? draft.stages[0] ?? null

  const select = (id: string) => {
    setSelectedId(id)
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
      setSelectedId(neighbour ? neighbour.id : null)
      setMobileDetail(false)
    }
  }

  const editor = selected ? (
    <StageEditor
      stage={selected}
      index={draft.stages.findIndex((s) => s.id === selected.id)}
      onChange={(next) => updateStage(selected.id, next)}
      onRemove={() => removeStage(selected.id)}
    />
  ) : (
    <EmptyDetail />
  )

  const rail = (
    <StepRail
      stages={draft.stages}
      selectedId={selected?.id ?? null}
      onSelect={select}
      onAddStage={addStage}
    />
  )

  if (isMobile) {
    return (
      <div className="rounded-lg border p-2">
        {mobileDetail && selected ? (
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
    <div className="flex min-h-[22rem] overflow-hidden rounded-lg border">
      <div className="w-64 shrink-0 overflow-y-auto border-r bg-muted/20 p-2">
        {rail}
      </div>
      <div className="min-w-0 flex-1 overflow-y-auto p-4">{editor}</div>
    </div>
  )
}

function EmptyDetail() {
  return (
    <div className="flex h-full flex-col items-center justify-center gap-2 py-10 text-center text-muted-foreground">
      <ListPlus className="h-6 w-6" />
      <p className="text-sm">Add a step from the list to start the pipeline.</p>
    </div>
  )
}
