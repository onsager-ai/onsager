import {
  Bot,
  CheckSquare,
  CircleDot,
  Gavel,
  GitPullRequest,
  Rocket,
  ShieldCheck,
  Terminal,
} from "lucide-react"
import type { WorkflowArtifactKind, WorkflowGateKind } from "@/lib/api"

export interface ArtifactKindMeta {
  value: WorkflowArtifactKind
  label: string
  shortLabel: string
  icon: typeof Bot
}

// Static fallback set (issue #102). The canonical list comes from
// `GET /api/workflow/kinds`; when that fetch fails (offline / dev without
// stiglab) the UI renders this list so the builder is still usable.
// Kind ids align with the registry's `workflow_builtin_types()`.
export const WORKFLOW_ARTIFACT_KINDS: ArtifactKindMeta[] = [
  {
    value: "Issue",
    label: "GitHub Issue",
    shortLabel: "Issue",
    icon: CircleDot,
  },
  {
    value: "PR",
    label: "GitHub Pull Request",
    shortLabel: "PR",
    icon: GitPullRequest,
  },
  {
    value: "Deployment",
    label: "Deployment",
    shortLabel: "Deploy",
    icon: Rocket,
  },
  {
    value: "Session",
    label: "Agent Session",
    shortLabel: "Session",
    icon: Terminal,
  },
]

const ARTIFACT_META_BY_VALUE = Object.fromEntries(
  WORKFLOW_ARTIFACT_KINDS.map((k) => [k.value, k]),
) as Record<string, ArtifactKindMeta>

// Legacy kind ids (pre-#102) — older workflows persisted `github-issue`
// / `github-pr` / `Spec` / `PullRequest` values. Map them onto the v1
// canonical set so the UI doesn't fall through to the "unknown kind"
// branch for real workflows that are already on disk.
const LEGACY_KIND_ALIASES: Record<string, string> = {
  "github-issue": "Issue",
  Spec: "Issue",
  "github-pr": "PR",
  PullRequest: "PR",
}

export function artifactKindMeta(value: WorkflowArtifactKind): ArtifactKindMeta {
  const canonical = LEGACY_KIND_ALIASES[value] ?? value
  return (
    ARTIFACT_META_BY_VALUE[canonical] ?? {
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
//
// Normalizes legacy `github-issue` on the input side so old persisted
// workflows keep working through the UI transformation.
export function outputArtifactKind(
  gate: WorkflowGateKind,
  input: WorkflowArtifactKind,
): WorkflowArtifactKind {
  const canonical = LEGACY_KIND_ALIASES[input] ?? input
  if (gate === "agent-session" && canonical === "Issue") return "PR"
  return canonical
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
