import { useState } from "react"
import { ChevronLeft } from "lucide-react"
import { useIsMobile } from "@/hooks/use-mobile"
import { Button } from "@/components/ui/button"
import type { GitHubAppInstallation, WorkflowGateKind, WorkflowStage } from "@/lib/api"
import { FlowRail } from "./FlowRail"
import { StageEditor } from "./StageEditor"
import { TriggerEditor } from "./TriggerEditor"
import { isTriggerReady, makeStage, type WorkflowDocument } from "./workflow-draft"

/** Which node the right-pane editor is bound to. Stages are addressed by id
 *  (not index) so the selection survives reorders and neighbour removals. */
export type Selection = { kind: "trigger" } | { kind: "stage"; id: string }

export interface FlowEditorProps {
  workspaceId: string
  installations: GitHubAppInstallation[]
  draft: WorkflowDocument
  onChange: (next: WorkflowDocument) => void
}

/**
 * Master-detail workflow builder. The left {@link FlowRail} shows the whole
 * pipeline and stays visible while the right pane edits the selected node —
 * replacing the previous card-stack-plus-slide-out-sheet, which hid the flow
 * behind an overlay every time you tuned a node. On mobile the two panes
 * collapse to a list→detail push (one column, same model).
 */
export function FlowEditor({
  workspaceId,
  installations,
  draft,
  onChange,
}: FlowEditorProps) {
  const isMobile = useIsMobile()
  const [selection, setSelection] = useState<Selection>(() => initialSelection(draft))
  const [mobileDetail, setMobileDetail] = useState(false)

  // Guard a stale stage selection (e.g. after a preset swap replaces every
  // stage): fall back to the first stage, else the trigger.
  const selectedStage =
    selection.kind === "stage"
      ? draft.stages.find((s) => s.id === selection.id)
      : undefined
  const effective: Selection =
    selection.kind === "stage" && !selectedStage
      ? draft.stages[0]
        ? { kind: "stage", id: draft.stages[0].id }
        : { kind: "trigger" }
      : selection

  const select = (next: Selection) => {
    setSelection(next)
    setMobileDetail(true)
  }

  const addStage = (gate: WorkflowGateKind) => {
    const stage = makeStage(gate)
    onChange({ ...draft, stages: [...draft.stages, stage] })
    select({ kind: "stage", id: stage.id })
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
    if (selection.kind === "stage" && selection.id === id) {
      const neighbour = stages[idx] ?? stages[idx - 1]
      setSelection(neighbour ? { kind: "stage", id: neighbour.id } : { kind: "trigger" })
      setMobileDetail(false)
    }
  }

  const editor =
    effective.kind === "trigger" ? (
      <TriggerEditor
        workspaceId={workspaceId}
        installations={installations}
        value={draft.trigger}
        onChange={(trigger) => onChange({ ...draft, trigger })}
      />
    ) : (
      (() => {
        const index = draft.stages.findIndex((s) => s.id === effective.id)
        const stage = draft.stages[index]
        if (!stage) return null
        return (
          <StageEditor
            stage={stage}
            index={index}
            onChange={(next) => updateStage(stage.id, next)}
            onRemove={() => removeStage(stage.id)}
          />
        )
      })()
    )

  const rail = (
    <FlowRail
      draft={draft}
      selection={effective}
      onSelect={select}
      onAddStage={addStage}
    />
  )

  if (isMobile) {
    return (
      <div className="rounded-lg border p-2">
        {mobileDetail ? (
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

function initialSelection(draft: WorkflowDocument): Selection {
  // Land where setup is needed: an unconfigured trigger first, otherwise the
  // first stage, otherwise the trigger.
  if (!isTriggerReady(draft.trigger)) return { kind: "trigger" }
  if (draft.stages[0]) return { kind: "stage", id: draft.stages[0].id }
  return { kind: "trigger" }
}
