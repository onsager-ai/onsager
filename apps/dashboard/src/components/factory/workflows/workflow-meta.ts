import {
  Bot,
  CheckSquare,
  CircleDot,
  Gavel,
  GitPullRequest,
  ShieldCheck,
} from "lucide-react"
import type { WorkflowArtifactKind, WorkflowGateKind } from "@/lib/api"

export interface ArtifactKindMeta {
  value: WorkflowArtifactKind
  label: string
  shortLabel: string
  icon: typeof Bot
}

export const WORKFLOW_ARTIFACT_KINDS: ArtifactKindMeta[] = [
  {
    value: "github-issue",
    label: "GitHub Issue",
    shortLabel: "Issue",
    icon: CircleDot,
  },
  {
    value: "github-pr",
    label: "GitHub Pull Request",
    shortLabel: "PR",
    icon: GitPullRequest,
  },
]

const ARTIFACT_META_BY_VALUE = Object.fromEntries(
  WORKFLOW_ARTIFACT_KINDS.map((k) => [k.value, k]),
) as Partial<Record<WorkflowArtifactKind, ArtifactKindMeta>>

export function artifactKindMeta(value: WorkflowArtifactKind): ArtifactKindMeta {
  return (
    ARTIFACT_META_BY_VALUE[value] ?? {
      value,
      label: value,
      shortLabel: value,
      icon: CircleDot,
    }
  )
}

// Derive the artifact kind a stage emits from its gate and input kind.
// Only agent-session can change the artifact kind (issue → PR). All other
// gates are inspect-and-forward, so the output kind equals the input.
export function outputArtifactKind(
  gate: WorkflowGateKind,
  input: WorkflowArtifactKind,
): WorkflowArtifactKind {
  if (gate === "agent-session" && input === "github-issue") return "github-pr"
  return input
}

export const GATE_KINDS: {
  value: WorkflowGateKind
  label: string
  description: string
  icon: typeof Bot
}[] = [
  {
    value: "agent-session",
    label: "Agent session",
    description: "Claude Code session runs until done.",
    icon: Bot,
  },
  {
    value: "external-check",
    label: "External check",
    description: "Wait for a CI check or external signal.",
    icon: CheckSquare,
  },
  {
    value: "governance",
    label: "Governance",
    description: "Synodic verdict decides pass/fail.",
    icon: Gavel,
  },
  {
    value: "manual-approval",
    label: "Manual approval",
    description: "A human clicks approve to proceed.",
    icon: ShieldCheck,
  },
]
