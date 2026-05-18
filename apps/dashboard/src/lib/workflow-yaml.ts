// YAML round-trip for `WorkflowDocument`. The Configuration view of the
// right-panel preview (spec #400) renders a draft as YAML so a power user
// can paste-edit, and parse failures surface inline rather than corrupting
// the draft. The format is canonical: serializer is total, parser is
// strict — anything that doesn't shape-validate as a workflow document
// returns a typed error instead of a partial draft.

import { parse, stringify } from "yaml"

import type {
  WorkflowDocument,
  WorkflowTriggerDraft,
} from "@/components/factory/workflows/workflow-draft"
import type {
  WorkflowArtifactKind,
  WorkflowGateKind,
  WorkflowStage,
} from "@/lib/api"

const GATE_KINDS: WorkflowGateKind[] = [
  "agent-session",
  "external-check",
  "governance",
  "manual-approval",
]

/** Round-trip the draft document as canonical YAML. */
export function workflowDocumentToYaml(doc: WorkflowDocument): string {
  return stringify(
    {
      name: doc.name,
      trigger: {
        install_id: doc.trigger.install_id,
        repo_owner: doc.trigger.repo_owner,
        repo_name: doc.trigger.repo_name,
        label: doc.trigger.label,
      },
      stages: doc.stages.map((s) => ({
        id: s.id,
        name: s.name,
        gate_kind: s.gate_kind,
        artifact_kind: s.artifact_kind,
        config: s.config,
      })),
    },
    { lineWidth: 0 },
  )
}

export class WorkflowYamlError extends Error {}

/**
 * Parse YAML text back into a `WorkflowDocument`. Throws
 * `WorkflowYamlError` with a human-readable message on any shape
 * mismatch — callers surface that as inline copy on the YAML side per
 * spec #400's "couldn't parse" path.
 */
export function workflowDocumentFromYaml(text: string): WorkflowDocument {
  let raw: unknown
  try {
    raw = parse(text)
  } catch (err) {
    throw new WorkflowYamlError(
      err instanceof Error ? err.message : "YAML parse failed",
    )
  }
  if (!isObject(raw)) {
    throw new WorkflowYamlError("Top-level YAML must be a mapping")
  }
  const name = readString(raw, "name")
  const trigger = parseTrigger(raw.trigger)
  const stages = parseStages(raw.stages)
  return { name, trigger, stages }
}

function parseTrigger(raw: unknown): WorkflowTriggerDraft {
  if (!isObject(raw)) {
    throw new WorkflowYamlError("`trigger` must be a mapping")
  }
  return {
    install_id: readString(raw, "install_id"),
    repo_owner: readString(raw, "repo_owner"),
    repo_name: readString(raw, "repo_name"),
    label: readString(raw, "label"),
  }
}

function parseStages(raw: unknown): WorkflowStage[] {
  if (!Array.isArray(raw)) {
    throw new WorkflowYamlError("`stages` must be a list")
  }
  return raw.map((entry, i) => {
    if (!isObject(entry)) {
      throw new WorkflowYamlError(`stage ${i} must be a mapping`)
    }
    const gateKind = readString(entry, "gate_kind") as WorkflowGateKind
    if (!GATE_KINDS.includes(gateKind)) {
      throw new WorkflowYamlError(
        `stage ${i} has unknown gate_kind \`${gateKind}\``,
      )
    }
    const artifactKind = readString(entry, "artifact_kind") as WorkflowArtifactKind
    const config = entry.config
    if (config != null && !isObject(config)) {
      throw new WorkflowYamlError(`stage ${i} \`config\` must be a mapping`)
    }
    return {
      id: readString(entry, "id"),
      name: readString(entry, "name"),
      gate_kind: gateKind,
      artifact_kind: artifactKind,
      config: (config ?? {}) as Record<string, unknown>,
    }
  })
}

function isObject(v: unknown): v is Record<string, unknown> {
  return typeof v === "object" && v != null && !Array.isArray(v)
}

function readString(obj: Record<string, unknown>, key: string): string {
  const v = obj[key]
  if (typeof v === "string") return v
  if (v == null) return ""
  return String(v)
}
