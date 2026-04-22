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

// Preset catalog — drives the picker shown at the top of the workflow
// builder. Each entry returns a fresh draft; the trigger stays empty and
// is filled in via the TriggerCard.
export interface WorkflowPreset {
  id: string
  label: string
  description: string
  build: (trigger: WorkflowTriggerDraft) => WorkflowDraft
}

export const WORKFLOW_PRESETS: WorkflowPreset[] = [
  {
    id: GITHUB_ISSUE_TO_PR_PRESET,
    label: "Issue → PR",
    description: "Agent opens a PR from an issue, waits on CI, then human merges.",
    build: githubIssueToPrPreset,
  },
  {
    id: "agent-only",
    label: "Agent only",
    description: "Run a single agent session on each triggered issue.",
    build: (trigger) => ({
      name: `${trigger.repo_owner || "agent"}/${trigger.repo_name || "session"} — agent only`,
      trigger,
      stages: [makeStage("agent-session", "github-issue", "Agent session")],
    }),
  },
  {
    id: "ci-then-merge",
    label: "CI → Merge",
    description: "Wait for CI to pass on a PR, then require a human to merge.",
    build: (trigger) => ({
      name: `${trigger.repo_owner || "repo"}/${trigger.repo_name || "pr"} — CI to merge`,
      trigger,
      stages: [
        makeStage("external-check", "github-pr", "CI check"),
        makeStage("manual-approval", "github-pr", "Merge approval"),
      ],
    }),
  },
  {
    id: "governed-pipeline",
    label: "Governed pipeline",
    description: "Agent builds, CI checks, Synodic governs, human merges.",
    build: (trigger) => ({
      name: `${trigger.repo_owner || "repo"}/${trigger.repo_name || "pipeline"} — governed`,
      trigger,
      stages: [
        makeStage("agent-session", "github-issue", "Spec → PR"),
        makeStage("external-check", "github-pr", "CI check"),
        makeStage("governance", "github-pr", "Synodic gate"),
        makeStage("manual-approval", "github-pr", "Merge approval"),
      ],
    }),
  },
]

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
