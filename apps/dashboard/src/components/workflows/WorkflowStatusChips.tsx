import { Button } from "@/components/ui/button"
import { cn } from "@/lib/utils"
import type {
  WorkflowStatusChip,
  WorkflowStatusCounts,
} from "@/lib/api"

// Workflow activation chips on the Workflows tab (#456 / ADR 0019).
// State values map to the spine's `workflows.active` + `install_id` via
// the portal's `WorkflowStatus` derivation. `Drafted` gets a light pulse
// when count > 0 — it's the activation-ladder key metric (#404), so
// "you have unbound drafts" surfaces without a drill-in.
const CHIPS: { key: WorkflowStatusChip; label: string }[] = [
  { key: "all", label: "All" },
  { key: "drafted", label: "Drafted" },
  { key: "bound", label: "Bound" },
  { key: "active", label: "Active" },
  { key: "paused", label: "Paused" },
]

interface WorkflowStatusChipsProps {
  value: WorkflowStatusChip
  counts: WorkflowStatusCounts
  onChange: (next: WorkflowStatusChip) => void
}

export function WorkflowStatusChips({
  value,
  counts,
  onChange,
}: WorkflowStatusChipsProps) {
  return (
    <div
      className="-mx-1 flex flex-wrap items-center gap-2 px-1"
      role="tablist"
      aria-label="Workflow status filter"
    >
      {CHIPS.map((chip) => {
        const count = counts[chip.key] ?? 0
        const active = value === chip.key
        const emphasize = chip.key === "drafted" && count > 0
        return (
          <Button
            key={chip.key}
            type="button"
            role="tab"
            aria-selected={active}
            size="sm"
            variant={active ? "default" : "outline"}
            className={cn(
              "h-7 rounded-full px-3 text-xs font-medium",
              emphasize && !active && "border-primary/40",
            )}
            onClick={() => onChange(chip.key)}
          >
            <span>{chip.label}</span>
            <span
              className={cn(
                "ml-1.5 inline-flex h-4 min-w-4 items-center justify-center rounded-full px-1 text-[10px]",
                active
                  ? "bg-primary-foreground/20 text-primary-foreground"
                  : "bg-muted text-muted-foreground",
                emphasize && !active && "bg-primary/10 text-primary",
              )}
              aria-label={`${count} ${chip.label.toLowerCase()}`}
            >
              {count}
            </span>
          </Button>
        )
      })}
    </div>
  )
}
