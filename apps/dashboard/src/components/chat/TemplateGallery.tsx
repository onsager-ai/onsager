import type { ComponentType } from "react"
import { GitBranch, GitMerge, Rocket, Shield, Sparkles } from "lucide-react"
import { Button } from "@/components/ui/button"
import {
  WORKFLOW_PRESETS,
  type WorkflowDocument,
  type WorkflowPreset,
} from "@/components/factory/workflows/workflow-draft"

// Six FTUE templates per spec #400's empty-state gallery. The preset
// catalog (workflow-draft.ts) supplies the underlying DAGs; this list
// maps each to evaluator-friendly framing (factory-metaphor short copy,
// icon, primary artifact). Until axis 9 ships the full template-library
// API (umbrella Open Question 9), the gallery reads from the dashboard's
// static preset catalog — one source of truth for both "create from
// scratch" (PresetPicker) and "FTUE workspace-less browse".
const FACE_BY_PRESET: Record<
  string,
  { headline: string; intent: string; icon: ComponentType<{ className?: string }> }
> = {
  "github-issue-to-pr": {
    headline: "Issue → PR",
    intent: "Agent picks up labeled issues and ships a reviewed PR.",
    icon: GitBranch,
  },
  "agent-only": {
    headline: "Agent only",
    intent: "Run one agent session on every triggered issue.",
    icon: Sparkles,
  },
  "ci-then-merge": {
    headline: "CI → Merge",
    intent: "Wait for CI to go green, then prompt a human to merge.",
    icon: GitMerge,
  },
  "governed-pipeline": {
    headline: "Governed pipeline",
    intent: "Spec → PR with Synodic governance and human sign-off.",
    icon: Shield,
  },
  "merge-to-deploy": {
    headline: "Merge → Deploy",
    intent: "Roll merged PRs to staging on every merge.",
    icon: Rocket,
  },
}

export interface TemplateGalleryProps {
  onPick: (
    preset: WorkflowPreset,
    workflow: WorkflowDocument,
  ) => void
}

export function TemplateGallery({ onPick }: TemplateGalleryProps) {
  return (
    <div className="flex w-full flex-col gap-2">
      <p className="px-1 text-xs font-medium text-muted-foreground">
        Start from a template or describe your own.
      </p>
      <div className="-mx-1 flex gap-2 overflow-x-auto px-1 pb-1">
        {WORKFLOW_PRESETS.map((preset) => {
          const face = FACE_BY_PRESET[preset.id] ?? {
            headline: preset.label,
            intent: preset.description,
            icon: Sparkles,
          }
          const Icon = face.icon
          // Use the preset builder with an empty trigger — FTUE users
          // don't yet have an install/repo to bind to. The binding flow
          // (axis 5) fills the trigger in at promote-to-real time.
          const workflow = preset.build({
            install_id: "",
            repo_owner: "",
            repo_name: "",
            label: "",
          })
          return (
            <Button
              key={preset.id}
              type="button"
              variant="outline"
              className="h-auto min-w-[200px] flex-col items-start gap-1.5 whitespace-normal p-3 text-left"
              onClick={() => onPick(preset, workflow)}
            >
              <div className="flex items-center gap-1.5 text-sm font-medium">
                <Icon className="h-3.5 w-3.5 text-muted-foreground" />
                {face.headline}
              </div>
              <span className="text-xs font-normal text-muted-foreground">
                {face.intent}
              </span>
            </Button>
          )
        })}
      </div>
    </div>
  )
}
