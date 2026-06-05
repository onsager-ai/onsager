import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import {
  WORKFLOW_PRESETS,
  type WorkflowDocument,
  type WorkflowPreset,
} from "./workflow-draft"

export interface PresetPickerProps {
  draft: WorkflowDocument
  onApply: (next: WorkflowDocument) => void
}

// Preset picker shown above the card stack. Applying a preset fills the
// stage list while keeping the trigger (install/repo/label) intact so the
// user doesn't have to re-pick a repo they already chose. De-emphasized to
// one compact line — the name + card stack are the focus, presets are a
// shortcut.
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

  const onValueChange = (id: string | null) => {
    const preset = WORKFLOW_PRESETS.find((p) => p.id === id)
    if (preset) apply(preset)
  }

  return (
    <div className="flex items-center gap-2 text-xs text-muted-foreground">
      <span className="shrink-0">Start from a preset</span>
      <Select value="" onValueChange={onValueChange}>
        <SelectTrigger size="sm" className="w-full" aria-label="Start from a preset">
          <SelectValue placeholder="Choose a preset…" />
        </SelectTrigger>
        <SelectContent>
          {WORKFLOW_PRESETS.map((p) => (
            <SelectItem key={p.id} value={p.id}>
              <span className="flex flex-col">
                <span className="text-sm font-medium text-foreground">{p.label}</span>
                <span className="text-xs text-muted-foreground">{p.description}</span>
              </span>
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
    </div>
  )
}
