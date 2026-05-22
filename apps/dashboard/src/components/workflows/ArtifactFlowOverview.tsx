import { ArrowRight, Tag } from "lucide-react"
import type { WorkflowGateKind, WorkflowStage } from "@/lib/api"
import { GATE_KINDS } from "./workflow-meta"

export interface ArtifactFlowOverviewProps {
  triggerLabel: string
  stages: WorkflowStage[]
  /// Optional current stage index for live runs. The matching pill gets
  /// the "in-flight" accent; the rest stay muted.
  currentStageIndex?: number
}

// Linear sequence of gate pills: Trigger → Agent → CI → Synodic → Human → …
//
// Before #104 this component rendered one pill per stage's input + output
// artifact kind, which duplicated the same artifact multiple times in a row
// ("Governed pipeline" showed Issue → PR → PR → PR → PR). Artifact state now
// lives in a separate `DeliverablePanel`; this strip is strictly "where are
// we in the process?" One pill per gate, no artifact duplication.
export function ArtifactFlowOverview({
  triggerLabel,
  stages,
  currentStageIndex,
}: ArtifactFlowOverviewProps) {
  if (stages.length === 0) {
    return (
      <p className="text-xs text-muted-foreground">
        Add a stage to see the pipeline flow.
      </p>
    )
  }

  const gateMeta = Object.fromEntries(
    GATE_KINDS.map((g) => [g.value, g]),
  ) as Record<WorkflowGateKind, (typeof GATE_KINDS)[number]>

  return (
    <div
      className="-mx-1 overflow-x-auto pb-1"
      data-testid="workflow-flow-strip"
    >
      <div className="flex min-w-max items-center gap-1.5 px-1">
        <span className="inline-flex items-center gap-1 rounded-full border border-dashed border-muted-foreground/30 bg-muted/40 px-2 py-0.5 text-xs text-muted-foreground">
          <Tag aria-hidden focusable={false} className="h-3 w-3" />
          {triggerLabel || "trigger"}
        </span>
        {stages.map((s, i) => {
          const meta = gateMeta[s.gate_kind]
          const Icon = meta?.icon
          const isCurrent = i === currentStageIndex
          return (
            <div key={s.id} className="flex items-center gap-1.5">
              <ArrowRight
                aria-hidden
                focusable={false}
                className="h-3 w-3 shrink-0 text-muted-foreground"
              />
              <span
                className={
                  isCurrent
                    ? "inline-flex items-center gap-1 rounded-full border border-primary/40 bg-primary/10 px-2 py-0.5 text-xs font-medium text-primary"
                    : "inline-flex items-center gap-1 rounded-full border border-muted-foreground/20 bg-muted/60 px-2 py-0.5 text-xs text-foreground"
                }
                title={meta?.label ?? s.gate_kind}
              >
                {Icon && (
                  <Icon aria-hidden focusable={false} className="h-3 w-3" />
                )}
                {s.name || meta?.label || s.gate_kind}
              </span>
            </div>
          )
        })}
      </div>
    </div>
  )
}
