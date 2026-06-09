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
} from "@/components/workflows/workflow-draft"
import type { WorkflowGateKind, WorkflowStage } from "@/lib/api"

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
        kind_tag: doc.trigger.kind_tag,
        install_id: doc.trigger.install_id,
        repo_owner: doc.trigger.repo_owner,
        repo_name: doc.trigger.repo_name,
        label: doc.trigger.label,
        manual_name: doc.trigger.manual_name,
      },
      stages: doc.stages.map((s) => ({
        id: s.id,
        name: s.name,
        gate_kind: s.gate_kind,
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
 * repo_owner, repo_name, label}`, every stage's `{id, name, gate_kind}`.
 * Empty strings are allowed (a half-filled draft still round-trips); missing
 * keys are not.
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
  // `kind_tag` / `manual_name` are read optionally so YAML authored before
  // the Manual trigger kind (#561) still round-trips: an absent `kind_tag`
  // defaults to the original `github_issue_webhook` behavior.
  return {
    kind_tag: optionalString(raw, "kind_tag", "trigger") || "github_issue_webhook",
    install_id: requireString(raw, "install_id", "trigger"),
    repo_owner: requireString(raw, "repo_owner", "trigger"),
    repo_name: requireString(raw, "repo_name", "trigger"),
    label: requireString(raw, "label", "trigger"),
    manual_name: optionalString(raw, "manual_name", "trigger"),
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
      config: (config ?? {}) as Record<string, unknown>,
    }
  })
}

function isObject(v: unknown): v is Record<string, unknown> {
  return typeof v === "object" && v != null && !Array.isArray(v)
}

// Like `requireString` but a missing key yields `""` rather than throwing.
// Used for fields added after the original YAML shape (e.g. the Manual
// trigger's `kind_tag` / `manual_name`, #561) so older drafts still parse.
function optionalString(
  obj: Record<string, unknown>,
  key: string,
  ctx: string,
): string {
  if (!(key in obj)) return ""
  return requireString(obj, key, ctx)
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
