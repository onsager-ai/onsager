import { Button } from "@/components/ui/button"
import { cn } from "@/lib/utils"
import type {
  AutofireFilter,
  ReadinessFilter,
  WorkflowAxisCounts,
} from "@/lib/api"

// Two independent axis filters on the Workflows tab (ADR 0024 / spec #509).
// `readiness` (Drafted / Ready) and `autofire` (Manual / Active / Paused)
// are orthogonal — selecting a value in each combines with AND server-side.
// Each group has an "All" chip that clears that axis. `Drafted` gets a
// light pulse when its count > 0 (the activation-ladder key metric, #404),
// so "you have unbound drafts" surfaces without a drill-in.
const READINESS_CHIPS: { key: ReadinessFilter; label: string }[] = [
  { key: "all", label: "All" },
  { key: "drafted", label: "Drafted" },
  { key: "ready", label: "Ready" },
]

const AUTOFIRE_CHIPS: { key: AutofireFilter; label: string }[] = [
  { key: "all", label: "All" },
  { key: "off", label: "Manual" },
  { key: "active", label: "Active" },
  { key: "paused", label: "Paused" },
]

interface WorkflowStatusChipsProps {
  readiness: ReadinessFilter
  autofire: AutofireFilter
  counts: WorkflowAxisCounts
  onReadinessChange: (next: ReadinessFilter) => void
  onAutofireChange: (next: AutofireFilter) => void
}

export function WorkflowStatusChips({
  readiness,
  autofire,
  counts,
  onReadinessChange,
  onAutofireChange,
}: WorkflowStatusChipsProps) {
  const readinessCount = (key: ReadinessFilter): number => {
    switch (key) {
      case "all":
        return counts.all
      case "drafted":
        return counts.readiness.drafted
      case "ready":
        return counts.readiness.ready
    }
  }

  const autofireCount = (key: AutofireFilter): number => {
    switch (key) {
      case "all":
        return counts.all
      case "off":
        return counts.autofire.off
      case "active":
        return counts.autofire.active
      case "paused":
        return counts.autofire.paused
    }
  }

  return (
    <div className="flex flex-col gap-2">
      <ChipRow
        ariaLabel="Workflow readiness filter"
        chips={READINESS_CHIPS}
        value={readiness}
        countFor={readinessCount}
        emphasizeKey="drafted"
        onChange={onReadinessChange}
      />
      <ChipRow
        ariaLabel="Workflow autofire filter"
        chips={AUTOFIRE_CHIPS}
        value={autofire}
        countFor={autofireCount}
        onChange={onAutofireChange}
      />
    </div>
  )
}

interface ChipRowProps<K extends string> {
  ariaLabel: string
  chips: { key: K; label: string }[]
  value: K
  countFor: (key: K) => number
  emphasizeKey?: K
  onChange: (next: K) => void
}

function ChipRow<K extends string>({
  ariaLabel,
  chips,
  value,
  countFor,
  emphasizeKey,
  onChange,
}: ChipRowProps<K>) {
  return (
    <div
      className="-mx-1 flex flex-wrap items-center gap-2 px-1"
      role="tablist"
      aria-label={ariaLabel}
    >
      {chips.map((entry) => {
        const count = countFor(entry.key)
        const isActive = value === entry.key
        const emphasize = entry.key === emphasizeKey && count > 0
        return (
          <Button
            key={entry.key}
            type="button"
            role="tab"
            aria-selected={isActive}
            size="sm"
            variant={isActive ? "default" : "outline"}
            className={cn(
              "h-7 rounded-full px-3 text-xs font-medium",
              emphasize && !isActive && "border-primary/40",
            )}
            onClick={() => onChange(entry.key)}
          >
            <span>{entry.label}</span>
            <span
              className={cn(
                "ml-1.5 inline-flex h-4 min-w-4 items-center justify-center rounded-full px-1 text-[10px]",
                isActive
                  ? "bg-primary-foreground/20 text-primary-foreground"
                  : "bg-muted text-muted-foreground",
                emphasize && !isActive && "bg-primary/10 text-primary",
              )}
              aria-label={`${count} ${entry.label.toLowerCase()}`}
            >
              {count}
            </span>
          </Button>
        )
      })}
    </div>
  )
}
