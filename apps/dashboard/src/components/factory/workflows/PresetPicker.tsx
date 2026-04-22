import { Sparkles } from "lucide-react"
import { Button } from "@/components/ui/button"
import { Card, CardContent } from "@/components/ui/card"
import {
  WORKFLOW_PRESETS,
  type WorkflowDraft,
  type WorkflowPreset,
} from "./workflow-draft"

export interface PresetPickerProps {
  draft: WorkflowDraft
  onApply: (next: WorkflowDraft) => void
}

// Preset picker shown above the chat builder. Applying a preset fills the
// stage list while keeping the trigger (install/repo/label) intact so the
// user doesn't have to re-pick a repo they already chose.
export function PresetPicker({ draft, onApply }: PresetPickerProps) {
  const apply = (preset: WorkflowPreset) => {
    const next = preset.build(draft.trigger)
    onApply({
      ...next,
      // Preserve a user-supplied name if they've already typed one; the
      // preset's generated name is a suggestion, not an override.
      name: draft.name.trim() !== "" ? draft.name : next.name,
    })
  }

  return (
    <Card>
      <CardContent className="flex flex-col gap-3 p-4">
        <div className="flex items-center gap-2 text-xs text-muted-foreground">
          <Sparkles className="h-3.5 w-3.5" />
          Start from a preset, or build from scratch below.
        </div>
        <div className="grid grid-cols-1 gap-2 sm:grid-cols-2">
          {WORKFLOW_PRESETS.map((p) => (
            <Button
              key={p.id}
              type="button"
              variant="outline"
              className="h-auto flex-col items-start gap-1 whitespace-normal p-3 text-left"
              onClick={() => apply(p)}
            >
              <span className="text-sm font-medium">{p.label}</span>
              <span className="text-xs font-normal text-muted-foreground">
                {p.description}
              </span>
            </Button>
          ))}
        </div>
      </CardContent>
    </Card>
  )
}
