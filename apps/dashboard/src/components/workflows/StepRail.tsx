import { Plus } from "lucide-react"
import { cn } from "@/lib/utils"
import type { WorkflowGateKind, WorkflowStage } from "@/lib/api"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { GATE_KINDS } from "./workflow-meta"

const GATE_ICON = Object.fromEntries(
  GATE_KINDS.map((g) => [g.value, g.icon]),
) as Record<WorkflowGateKind, (typeof GATE_KINDS)[number]["icon"]>

export interface StepRailProps {
  stages: WorkflowStage[]
  selectedId: string | null
  onSelect: (id: string) => void
  onAddStage: (gate: WorkflowGateKind) => void
}

/**
 * Left rail of the Steps master-detail: a vertical pipeline "spine", one row
 * per stage, with a typed "add step" dropdown at the bottom. Selecting a row
 * drives the right-pane {@link StageEditor}; the rail stays visible while you
 * edit. The trigger node no longer lives here — it moved to the Trigger tab —
 * so the rail is steps-only.
 */
export function StepRail({
  stages,
  selectedId,
  onSelect,
  onAddStage,
}: StepRailProps) {
  return (
    <nav aria-label="Workflow steps" className="flex flex-col">
      {stages.map((stage, i) => {
        const Icon = GATE_ICON[stage.gate_kind]
        return (
          <RailRow
            key={stage.id}
            marker={i + 1}
            title={
              <span className="flex min-w-0 items-center gap-1.5">
                <Icon className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
                <span className="truncate">{stage.name}</span>
              </span>
            }
            selected={selectedId === stage.id}
            onClick={() => onSelect(stage.id)}
            isFirst={i === 0}
          />
        )
      })}

      <DropdownMenu>
        <DropdownMenuTrigger
          render={
            <button
              type="button"
              className="group flex w-full items-stretch gap-2 text-left"
            />
          }
        >
          <RailMarker tone="add" isFirst={stages.length === 0} isLast>
            <Plus className="h-3.5 w-3.5" />
          </RailMarker>
          <span className="flex-1 rounded-md px-3 py-2.5 text-sm font-medium text-muted-foreground transition group-hover:bg-muted/50">
            Add step
          </span>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="start" className="w-64">
          {GATE_KINDS.map((g) => (
            <DropdownMenuItem
              key={g.value}
              className="flex-col items-start gap-0.5"
              onClick={() => onAddStage(g.value)}
            >
              <span className="flex items-center gap-2 text-sm font-medium">
                <g.icon className="h-4 w-4" />
                {g.label}
              </span>
              <span className="text-xs text-muted-foreground">{g.description}</span>
            </DropdownMenuItem>
          ))}
        </DropdownMenuContent>
      </DropdownMenu>

      {stages.length === 0 && (
        <p className="px-3 pt-1 text-xs text-muted-foreground">
          Add at least one step to run this workflow.
        </p>
      )}
    </nav>
  )
}

function RailRow({
  marker,
  title,
  selected,
  onClick,
  isFirst,
}: {
  marker: React.ReactNode
  title: React.ReactNode
  selected?: boolean
  onClick: () => void
  isFirst?: boolean
}) {
  return (
    <button
      type="button"
      aria-current={selected ? "true" : undefined}
      onClick={onClick}
      className="group flex w-full items-stretch gap-2 text-left"
    >
      <RailMarker tone="stage" selected={selected} isFirst={isFirst}>
        {marker}
      </RailMarker>
      <span
        className={cn(
          "min-w-0 flex-1 space-y-1 rounded-md px-3 py-2.5 transition",
          selected ? "bg-accent" : "group-hover:bg-muted/50",
        )}
      >
        <span className="flex min-w-0 items-center text-sm font-medium">{title}</span>
      </span>
    </button>
  )
}

// The marker column draws the connecting spine line plus the node dot. The
// line spans the full row height except at the two ends (trimmed to a half so
// the spine starts/stops at the first/last dot).
function RailMarker({
  tone,
  selected,
  isFirst,
  isLast,
  children,
}: {
  tone: "stage" | "add"
  selected?: boolean
  isFirst?: boolean
  isLast?: boolean
  children: React.ReactNode
}) {
  return (
    <span aria-hidden className="relative w-9 shrink-0">
      <span
        className={cn(
          "absolute left-[18px] w-px -translate-x-1/2 bg-border",
          isFirst ? "top-1/2" : "top-0",
          isLast ? "bottom-1/2" : "bottom-0",
        )}
      />
      <span
        className={cn(
          "absolute left-[18px] top-1/2 z-10 flex h-6 w-6 -translate-x-1/2 -translate-y-1/2 items-center justify-center rounded-full border text-[11px] font-semibold",
          selected
            ? "border-primary bg-primary text-primary-foreground"
            : tone === "add"
              ? "border-dashed bg-background text-muted-foreground group-hover:border-primary/40 group-hover:text-primary"
              : "border-border bg-background text-muted-foreground",
        )}
      >
        {children}
      </span>
    </span>
  )
}
