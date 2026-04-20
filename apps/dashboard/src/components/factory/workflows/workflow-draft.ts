import type {
  WorkflowArtifactKind,
  WorkflowGateKind,
  WorkflowStage,
  WorkflowTrigger,
} from "@/lib/api"

// The in-memory shape the chat/card editor manipulates. Everything here is
// structured — no free-text linkable fields. The card stack is the source
// of truth; the chat builder emits tool-call proposals that merge into
// this draft.
export interface WorkflowDraft {
  name: string
  trigger: WorkflowTriggerDraft
  stages: WorkflowStage[]
}

export interface WorkflowTriggerDraft {
  install_id: string
  repo_owner: string
  repo_name: string
  label: string
}

export const GITHUB_ISSUE_TO_PR_PRESET = "github-issue-to-pr" as const

function newStageId(): string {
  if (typeof crypto !== "undefined" && "randomUUID" in crypto) {
    return crypto.randomUUID()
  }
  return `stage_${Math.random().toString(36).slice(2, 10)}`
}

export function makeStage(
  gate: WorkflowGateKind,
  artifactKind: WorkflowArtifactKind = "github-issue",
  name?: string,
): WorkflowStage {
  return {
    id: newStageId(),
    name: name ?? defaultStageName(gate),
    gate_kind: gate,
    artifact_kind: artifactKind,
    config: {},
  }
}

export function defaultStageName(gate: WorkflowGateKind): string {
  switch (gate) {
    case "agent-session":
      return "Agent session"
    case "external-check":
      return "CI check"
    case "governance":
      return "Governance"
    case "manual-approval":
      return "Manual approval"
  }
}

export function emptyDraft(): WorkflowDraft {
  return {
    name: "",
    trigger: { install_id: "", repo_owner: "", repo_name: "", label: "" },
    stages: [],
  }
}

export function githubIssueToPrPreset(
  trigger: WorkflowTriggerDraft,
): WorkflowDraft {
  return {
    name: `${trigger.repo_owner}/${trigger.repo_name} — issue to PR`,
    trigger,
    stages: [
      makeStage("agent-session", "github-issue", "Spec → PR"),
      makeStage("external-check", "github-pr", "CI check"),
      makeStage("manual-approval", "github-pr", "Merge approval"),
    ],
  }
}

export function isTriggerReady(t: WorkflowTriggerDraft): boolean {
  return (
    t.install_id.trim() !== "" &&
    t.repo_owner.trim() !== "" &&
    t.repo_name.trim() !== "" &&
    t.label.trim() !== ""
  )
}

export function draftToRequestTrigger(t: WorkflowTriggerDraft): WorkflowTrigger {
  return {
    kind: "github-label",
    install_id: t.install_id,
    repo_owner: t.repo_owner,
    repo_name: t.repo_name,
    label: t.label,
  }
}
