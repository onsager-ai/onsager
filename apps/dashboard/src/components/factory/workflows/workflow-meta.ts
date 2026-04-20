import { Bot, CheckSquare, Gavel, ShieldCheck } from "lucide-react"
import type { WorkflowArtifactKind, WorkflowGateKind } from "@/lib/api"

export const WORKFLOW_ARTIFACT_KINDS: { value: WorkflowArtifactKind; label: string }[] = [
  { value: "github-issue", label: "GitHub Issue" },
  { value: "github-pr", label: "GitHub Pull Request" },
]

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
