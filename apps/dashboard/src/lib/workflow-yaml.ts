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
 * Parse YAML text back into a `WorkflowDocument`. Strict: throws
 * `WorkflowYamlError` with a human-readable message on any shape
 * mismatch — missing required fields, non-string scalars where strings
 * are required, unknown gate kinds. Callers surface the message as
 * inline copy on the YAML side per spec #400's "couldn't parse" path.
 *
 * "Required" mirrors the WorkflowDocument shape: `name`, `trigger.{install_id,
 * repo_owner, repo_name, label}`, every stage's `{id, name, gate_kind,
 * artifact_kind}`. Empty strings are allowed (a half-filled draft still
 * round-trips); missing keys are not.
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
  const name = requireString(raw, "name", "top-level")
  const trigger = parseTrigger(raw.trigger)
  const stages = parseStages(raw.stages)
  return { name, trigger, stages }
}

function parseTrigger(raw: unknown): WorkflowTriggerDraft {
  if (!isObject(raw)) {
    throw new WorkflowYamlError("`trigger` must be a mapping")
  }
  return {
    install_id: requireString(raw, "install_id", "trigger"),
    repo_owner: requireString(raw, "repo_owner", "trigger"),
    repo_name: requireString(raw, "repo_name", "trigger"),
    label: requireString(raw, "label", "trigger"),
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
    const ctx = `stage ${i}`
    const gateKindRaw = requireString(entry, "gate_kind", ctx)
    if (!GATE_KINDS.includes(gateKindRaw as WorkflowGateKind)) {
      throw new WorkflowYamlError(
        `${ctx} has unknown gate_kind \`${gateKindRaw}\``,
      )
    }
    const config = entry.config
    if (config != null && !isObject(config)) {
      throw new WorkflowYamlError(`${ctx} \`config\` must be a mapping`)
    }
    return {
      id: requireString(entry, "id", ctx),
      name: requireString(entry, "name", ctx),
      gate_kind: gateKindRaw as WorkflowGateKind,
      artifact_kind: requireString(
        entry,
        "artifact_kind",
        ctx,
      ) as WorkflowArtifactKind,
      config: (config ?? {}) as Record<string, unknown>,
    }
  })
}

function isObject(v: unknown): v is Record<string, unknown> {
  return typeof v === "object" && v != null && !Array.isArray(v)
}

function requireString(
  obj: Record<string, unknown>,
  key: string,
  ctx: string,
): string {
  if (!(key in obj)) {
    throw new WorkflowYamlError(`${ctx} is missing \`${key}\``)
  }
  const v = obj[key]
  if (typeof v === "string") return v
  // Empty strings round-trip via `yaml` as either `""` or as the empty
  // scalar (`null`); accept both so a half-filled draft can be edited
  // without forcing the user to type `""` literals.
  if (v == null) return ""
  throw new WorkflowYamlError(
    `${ctx}.\`${key}\` must be a string (got ${typeof v})`,
  )
}
