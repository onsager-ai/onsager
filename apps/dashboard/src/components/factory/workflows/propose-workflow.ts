import type { WorkflowGateKind } from "@/lib/api"
import { makeStage, type WorkflowDraft } from "./workflow-draft"

// The structured output of the `proposeWorkflow` tool call. NL prompts are
// not persisted (resolved in #79); only the tool call outputs feed into
// the card stack.
export interface ProposeWorkflowCall {
  name?: string
  stages?: {
    name?: string
    gate_kind: WorkflowGateKind
    artifact_kind?: string
  }[]
}

// Heuristic proposer — real agent will replace this with an LLM tool
// call. The heuristic is deliberately dumb; it keeps the UX wired end-to-
// end so the card stack stays the source of truth.
export function proposePlaceholder(text: string): ProposeWorkflowCall {
  const lower = text.toLowerCase()
  const stages: NonNullable<ProposeWorkflowCall["stages"]> = []
  if (/agent|claude|session|implement|spec/.test(lower)) {
    stages.push({
      name: "Agent session",
      gate_kind: "agent-session",
      artifact_kind: "Issue",
    })
  }
  if (/ci|check|build|test/.test(lower)) {
    stages.push({
      name: "CI check",
      gate_kind: "external-check",
      artifact_kind: "PR",
    })
  }
  if (/govern/.test(lower)) {
    stages.push({
      name: "Governance",
      gate_kind: "governance",
      artifact_kind: "PR",
    })
  }
  if (/merge|approve|manual|human/.test(lower)) {
    stages.push({
      name: "Manual approval",
      gate_kind: "manual-approval",
      artifact_kind: "PR",
    })
  }
  return { stages }
}

export function applyProposal(
  draft: WorkflowDraft,
  call: ProposeWorkflowCall,
): WorkflowDraft {
  const stages = (call.stages ?? []).map((s) =>
    makeStage(s.gate_kind, s.artifact_kind ?? "Issue", s.name),
  )
  return {
    ...draft,
    name: call.name ?? draft.name,
    stages: stages.length > 0 ? stages : draft.stages,
  }
}
