import { ArrowRight, Tag } from "lucide-react"
import type { WorkflowStage } from "@/lib/api"
import { ArtifactBadge } from "./ArtifactBadge"
import { outputArtifactKind } from "./workflow-meta"

export interface ArtifactFlowOverviewProps {
  triggerLabel: string
  stages: WorkflowStage[]
}

// A one-line visual summary of what enters each stage and what leaves
// the workflow — starts with the trigger label pill, then input→output
// badges for each stage separated by arrows. Scrolls horizontally on
// mobile rather than wrapping so the pipeline reads left to right.
export function ArtifactFlowOverview({
  triggerLabel,
  stages,
}: ArtifactFlowOverviewProps) {
  if (stages.length === 0) {
    return (
      <p className="text-xs text-muted-foreground">
        Add a stage to see how artifacts flow.
      </p>
    )
  }

  return (
    <div className="-mx-1 overflow-x-auto pb-1">
      <div className="flex min-w-max items-center gap-1.5 px-1">
        <span className="inline-flex items-center gap-1 rounded-full border border-dashed border-muted-foreground/30 bg-muted/40 px-2 py-0.5 text-xs text-muted-foreground">
          <Tag aria-hidden focusable={false} className="h-3 w-3" />
          {triggerLabel || "trigger"}
        </span>
        {stages.map((s, i) => {
          const input = s.artifact_kind
          const output = outputArtifactKind(s.gate_kind, input)
          const prevOutput =
            i === 0
              ? null
              : outputArtifactKind(stages[i - 1].gate_kind, stages[i - 1].artifact_kind)
          const transforms = output !== input
          return (
            <div key={s.id} className="flex items-center gap-1.5">
              <ArrowRight
                aria-hidden
                focusable={false}
                className="h-3 w-3 shrink-0 text-muted-foreground"
              />
              <ArtifactBadge
                kind={input}
                variant={prevOutput && prevOutput !== input ? "muted" : "default"}
              />
              {transforms && (
                <>
                  <ArrowRight
                    aria-hidden
                    focusable={false}
                    className="h-3 w-3 shrink-0 text-muted-foreground"
                  />
                  <ArtifactBadge kind={output} />
                </>
              )}
            </div>
          )
        })}
      </div>
    </div>
  )
}
