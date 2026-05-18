import type {
  WorkflowArtifactKind,
  WorkflowGateKind,
} from "@/lib/api/types"
import type { WorkflowDocument } from "@/components/factory/workflows/workflow-draft"
import { makeStage } from "@/components/factory/workflows/workflow-draft"
import templatesData from "./v0.json"

export interface FtueTemplateStage {
  name: string
  gate_kind: WorkflowGateKind
  artifact_kind: WorkflowArtifactKind
}

export interface FtueTemplate {
  id: string
  name: string
  scenario_class: "A" | "B" | "C" | "D"
  intent: string
  trigger_kind: string
  trigger_label: string
  stages: FtueTemplateStage[]
  primary_artifact_kind: WorkflowArtifactKind
  factory_framing: string
  cloud_only_note?: string
}

interface TemplateManifest {
  version: number
  templates: FtueTemplate[]
}

const manifest = templatesData as TemplateManifest

export const TEMPLATES: FtueTemplate[] = manifest.templates

export function getTemplate(id: string): FtueTemplate | undefined {
  return TEMPLATES.find((t) => t.id === id)
}

// Project a template into a fresh editable WorkflowDocument. The outer
// WorkflowDraft record (source: "template", template_id, timestamps) is
// composed by the caller via useWorkflowDraft.newDraft. The repo-scoped
// fields (`install_id`, `repo_owner`, `repo_name`) stay blank because
// binding (#402) is what picks the workspace, install, and project.
// The `label` field carries whatever the template's trigger shape
// contributes (GitHub label name, cron expression, etc.) so the data
// round-trips into binding rather than being silently dropped — once
// the WorkflowTriggerDraft type grows non-GitHub trigger fields (#402),
// the cron expression moves into its proper slot.
export function templateToDocument(template: FtueTemplate): WorkflowDocument {
  return {
    name: template.name,
    trigger: {
      install_id: "",
      repo_owner: "",
      repo_name: "",
      label: template.trigger_label,
    },
    stages: template.stages.map((s) =>
      makeStage(s.gate_kind, s.artifact_kind, s.name),
    ),
  }
}
