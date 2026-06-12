import { Button } from "@/components/ui/button"
import type { WorkflowGateKind } from "@/lib/api"
import { GATE_KINDS } from "./workflow-meta"

export interface GateKindToggleProps {
  value: WorkflowGateKind
  onChange: (value: WorkflowGateKind) => void
}

// Rendered as a row of independently-focusable toggle buttons with
// `aria-pressed`. We deliberately avoid ARIA `radiogroup`/`radio` — those
// roles require roving tabindex + arrow-key navigation that a plain
// `<Button>` doesn't provide. Toggle-button semantics still convey the
// mutually-exclusive selection (only one is pressed at a time) without
// misleading assistive tech about keyboard behavior.
export function GateKindToggle({ value, onChange }: GateKindToggleProps) {
  return (
    <div className="grid grid-cols-2 gap-2" role="group" aria-label="Gate kind">
      {GATE_KINDS.map((g) => {
        const active = g.value === value
        return (
          <Button
            key={g.value}
            type="button"
            variant={active ? "default" : "outline"}
            aria-pressed={active}
            className="h-auto flex-col items-start gap-1 whitespace-normal px-3 py-2 text-left"
            onClick={() => onChange(g.value)}
          >
            <span className="flex items-center gap-2 text-sm font-medium">
              <g.icon className="h-4 w-4" />
              {g.label}
            </span>
            <span className="text-xs font-normal text-muted-foreground">
              {g.description}
            </span>
          </Button>
        )
      })}
    </div>
  )
}
