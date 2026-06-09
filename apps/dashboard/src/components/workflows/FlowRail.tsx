import { Plus, Webhook, Zap } from "lucide-react"
import { cn } from "@/lib/utils"
import type { WorkflowGateKind } from "@/lib/api"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { GATE_KINDS } from "./workflow-meta"
import {
  triggerRepos,
  type WorkflowDocument,
  type WorkflowTriggerDraft,
} from "./workflow-draft"
import type { Selection } from "./FlowEditor"

const GATE_ICON = Object.fromEntries(
  GATE_KINDS.map((g) => [g.value, g.icon]),
) as Record<WorkflowGateKind, (typeof GATE_KINDS)[number]["icon"]>

export interface FlowRailProps {
  draft: WorkflowDocument
  selection: Selection
  onSelect: (selection: Selection) => void
  onAddStage: (gate: WorkflowGateKind) => void
}

/**
 * The left rail of the master-detail builder: a vertical pipeline "spine"
 * with the trigger node on top, one row per stage, and a typed "add step"
 * dropdown at the bottom. Selecting a row drives the right-pane editor; the
 * rail stays visible while you edit so the whole flow is never hidden behind
 * an overlay (the old card→sheet pattern it replaces).
 */
export function FlowRail({ draft, selection, onSelect, onAddStage }: FlowRailProps) {
  const trigger = summarizeTrigger(draft.trigger)
  const stageCount = draft.stages.length

  return (
    <nav aria-label="Workflow steps" className="flex flex-col">
      <RailRow
        marker={
          draft.trigger.kind_tag === "manual" ? (
            <Zap className="h-3.5 w-3.5" />
          ) : (
            <Webhook className="h-3.5 w-3.5" />
          )
        }
        tone="trigger"
        title={trigger.kindLabel}
        subtitle={
          <span className="truncate">
            {trigger.repoSummary}
            {trigger.labelSummary ? ` · ${trigger.labelSummary}` : ""}
          </span>
        }
        selected={selection.kind === "trigger"}
        onClick={() => onSelect({ kind: "trigger" })}
        isFirst
      />

      {draft.stages.map((stage, i) => {
        const Icon = GATE_ICON[stage.gate_kind]
        return (
          <RailRow
            key={stage.id}
            marker={i + 1}
            tone="stage"
            title={
              <span className="flex min-w-0 items-center gap-1.5">
                <Icon className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
                <span className="truncate">{stage.name}</span>
              </span>
            }
            selected={selection.kind === "stage" && selection.id === stage.id}
            onClick={() => onSelect({ kind: "stage", id: stage.id })}
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
          <RailMarker tone="add" isLast>
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

      {stageCount === 0 && (
        <p className="px-3 pt-1 text-xs text-muted-foreground">
          Add at least one step to run this trigger.
        </p>
      )}
    </nav>
  )
}

type Tone = "trigger" | "stage" | "add"

function RailRow({
  marker,
  tone,
  title,
  subtitle,
  selected,
  onClick,
  isFirst,
  isLast,
}: {
  marker: React.ReactNode
  tone: Tone
  title: React.ReactNode
  subtitle?: React.ReactNode
  selected?: boolean
  onClick: () => void
  isFirst?: boolean
  isLast?: boolean
}) {
  return (
    <button
      type="button"
      aria-current={selected ? "true" : undefined}
      onClick={onClick}
      className="group flex w-full items-stretch gap-2 text-left"
    >
      <RailMarker tone={tone} selected={selected} isFirst={isFirst} isLast={isLast}>
        {marker}
      </RailMarker>
      <span
        className={cn(
          "min-w-0 flex-1 space-y-1 rounded-md px-3 py-2.5 transition",
          selected ? "bg-accent" : "group-hover:bg-muted/50",
        )}
      >
        <span className="flex min-w-0 items-center text-sm font-medium">{title}</span>
        {subtitle != null && (
          <span className="flex min-w-0 items-center text-xs text-muted-foreground">
            {subtitle}
          </span>
        )}
      </span>
    </button>
  )
}

// The marker column draws the connecting spine line plus the node dot. The
// line spans the full row height except at the two ends (trimmed to a half
// so the spine starts/stops at the first/last dot).
function RailMarker({
  tone,
  selected,
  isFirst,
  isLast,
  children,
}: {
  tone: Tone
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
            : tone === "trigger"
              ? "border-primary/40 bg-primary/10 text-primary"
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

function summarizeTrigger(t: WorkflowTriggerDraft): {
  kindLabel: string
  repoSummary: string
  labelSummary: string | null
} {
  if (t.kind_tag === "manual") {
    return {
      kindLabel: t.manual_name.trim()
        ? `Manual · ${t.manual_name.trim()}`
        : "Manual trigger",
      repoSummary: "Any repository in this workspace",
      labelSummary: null,
    }
  }
  const repos = triggerRepos(t)
  const labelSummary = t.label ? `Label: ${t.label}` : "Pick a trigger label"
  if (repos.length === 0) {
    return {
      kindLabel: "GitHub issue label",
      repoSummary: "Pick one or more repositories",
      labelSummary,
    }
  }
  const primary = repos[0]
  const repoSummary =
    repos.length === 1
      ? `${primary.repo_owner}/${primary.repo_name}`
      : `${primary.repo_owner}/${primary.repo_name} +${repos.length - 1} more`
  return { kindLabel: "GitHub issue label", repoSummary, labelSummary }
}
