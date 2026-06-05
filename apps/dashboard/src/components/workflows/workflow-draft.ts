import {
  ApiError,
  stageToCreateStage,
  type CreateWorkflowRequest,
  type GitHubAppInstallation,
  type WorkflowArtifactKind,
  type WorkflowGateKind,
  type WorkflowStage,
  type WorkflowTrigger,
} from "@/lib/api";

// The in-memory shape the chat/card editor manipulates. Everything here is
// structured — no free-text linkable fields. The card stack is the source
// of truth; the chat builder emits tool-call proposals that merge into
// this draft.
//
// Renamed from `WorkflowDraft` per spec #401: the bare `{name, trigger,
// stages}` shape is the *document* the user is authoring; the persisted
// `WorkflowDraft` record (spec #401) wraps a `WorkflowDocument` with the
// draft's identity (`id`, `user_id`, `source`, lifecycle timestamps,
// optional `bound_to`).
export interface WorkflowDocument {
  name: string;
  trigger: WorkflowTriggerDraft;
  stages: WorkflowStage[];
}

export interface WorkflowTriggerDraft {
  /**
   * Snake-case registry `kind_tag` for the selected trigger kind (e.g.
   * `'github_issue_webhook'`, `'manual'`). Drives `isTriggerReady`'s
   * per-kind readiness branch and which `TriggerKind` variant
   * `documentToCreateRequest` emits. Defaults to `'github_issue_webhook'`
   * (the original, GitHub-webhook-only behavior — see `emptyDocument`).
   */
  kind_tag: string;
  install_id: string;
  repo_owner: string;
  repo_name: string;
  label: string;
  /**
   * Manual-trigger button name; used only when `kind_tag === 'manual'`.
   * Empty for every other kind.
   */
  manual_name: string;
}

/**
 * One end-user-owned workflow draft, persisted client-side per spec #401.
 *
 * Drafts live in localStorage keyed by `onsager.drafts.<user_id>` until
 * the binding flow (axis 5) promotes them into a real spine workflow.
 * Server-side draft sync is a v1.5 follow-up — the substrate stays
 * untouched.
 */
export interface WorkflowDraft {
  id: string;
  user_id: string;
  /** Display name for the draft; defaults to "Untitled draft". */
  name: string;
  source: WorkflowDraftSource;
  /** Set when `source === "template"`. */
  template_id?: string;
  /** The in-progress workflow document being authored. */
  workflow: WorkflowDocument;
  /** ISO timestamp of first creation. */
  created_at: string;
  /** ISO timestamp written on every edit. */
  updated_at: string;
  /** Populated when the binding flow (axis 5) completes. */
  bound_to?: {
    workspace_id: string;
    workflow_id: string;
    bound_at: string;
  };
}

export type WorkflowDraftSource = "template" | "chat" | "yaml-paste" | "blank";

export const GITHUB_ISSUE_TO_PR_PRESET = "github-issue-to-pr" as const;

function newStageId(): string {
  if (typeof crypto !== "undefined" && "randomUUID" in crypto) {
    return crypto.randomUUID();
  }
  return `stage_${Math.random().toString(36).slice(2, 10)}`;
}

export function makeStage(
  gate: WorkflowGateKind,
  artifactKind: WorkflowArtifactKind = "Issue",
  name?: string,
): WorkflowStage {
  return {
    id: newStageId(),
    name: name ?? defaultStageName(gate),
    gate_kind: gate,
    artifact_kind: artifactKind,
    config: {},
  };
}

export function defaultStageName(gate: WorkflowGateKind): string {
  switch (gate) {
    case "agent-session":
      return "Agent session";
    case "external-check":
      return "CI check";
    case "governance":
      return "Governance";
    case "manual-approval":
      return "Manual approval";
  }
}

export function emptyDocument(): WorkflowDocument {
  return {
    name: "",
    trigger: {
      kind_tag: "github_issue_webhook",
      install_id: "",
      repo_owner: "",
      repo_name: "",
      label: "",
      manual_name: "",
    },
    stages: [],
  };
}

export function githubIssueToPrPreset(
  trigger: WorkflowTriggerDraft,
): WorkflowDocument {
  const owner = trigger.repo_owner || "repo";
  const name = trigger.repo_name || "issue-to-pr";
  return {
    name: `${owner}/${name} — issue to PR`,
    trigger,
    stages: [
      makeStage("agent-session", "Issue", "Spec → PR"),
      makeStage("external-check", "PR", "CI check"),
      makeStage("manual-approval", "PR", "Merge approval"),
    ],
  };
}

// Preset catalog — drives the picker shown at the top of the workflow
// builder. Each entry returns a fresh document; the trigger stays empty
// and is filled in via the TriggerCard.
export interface WorkflowPreset {
  id: string;
  label: string;
  description: string;
  build: (trigger: WorkflowTriggerDraft) => WorkflowDocument;
}

export const WORKFLOW_PRESETS: WorkflowPreset[] = [
  {
    id: GITHUB_ISSUE_TO_PR_PRESET,
    label: "Issue → PR",
    description:
      "Agent opens a PR from an issue, waits on CI, then human merges.",
    build: githubIssueToPrPreset,
  },
  {
    id: "agent-only",
    label: "Agent only",
    description: "Run a single agent session on each triggered issue.",
    build: (trigger) => ({
      name: `${trigger.repo_owner || "agent"}/${trigger.repo_name || "session"} — agent only`,
      trigger,
      stages: [makeStage("agent-session", "Issue", "Agent session")],
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
        makeStage("external-check", "PR", "CI check"),
        makeStage("manual-approval", "PR", "Merge approval"),
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
        makeStage("agent-session", "Issue", "Spec → PR"),
        makeStage("external-check", "PR", "CI check"),
        makeStage("governance", "PR", "Synodic gate"),
        makeStage("manual-approval", "PR", "Merge approval"),
      ],
    }),
  },
  {
    id: "merge-to-deploy",
    label: "Merge → Deploy staging",
    description:
      "Wait for a PR to merge, then roll the commit to staging. Requires a Deployment adapter.",
    build: (trigger) => ({
      name: `${trigger.repo_owner || "repo"}/${trigger.repo_name || "deploy"} — merge to deploy`,
      trigger,
      stages: [
        makeStage("external-check", "PR", "Merge wait"),
        makeStage("agent-session", "Deployment", "Deploy to staging"),
      ],
    }),
  },
];

export function isTriggerReady(t: WorkflowTriggerDraft): boolean {
  // Manual triggers (#561) are repo-less: a non-empty button name is the
  // only requirement — no install, repo, or label. The GitHub webhook leg
  // stays exactly as it was (#545: webhook repo is required, no regression).
  if (t.kind_tag === "manual") {
    return t.manual_name.trim() !== "";
  }
  return (
    t.install_id.trim() !== "" &&
    t.repo_owner.trim() !== "" &&
    t.repo_name.trim() !== "" &&
    t.label.trim() !== ""
  );
}

export function draftToRequestTrigger(
  t: WorkflowTriggerDraft,
): WorkflowTrigger {
  return {
    kind: "github-label",
    install_id: t.install_id,
    repo_owner: t.repo_owner,
    repo_name: t.repo_name,
    label: t.label,
    kind_tag: t.kind_tag,
    manual_name: t.kind_tag === "manual" ? t.manual_name : "",
  };
}

// Build the stiglab-shaped create-workflow request from the UI draft. The
// two non-obvious pieces:
//
// 1. `draft.trigger.install_id` is the dashboard's installation *record id*
//    (`GitHubAppInstallation.id`, e.g. `inst_abc…`) — that's what the
//    `/github-installations/:install_id/{accessible-repos,labels}` endpoints
//    take. The workflow POST contract needs the numeric GitHub install id
//    (`GitHubAppInstallation.install_id: i64`), so we resolve it by record
//    id against the installations the page already has loaded.
// 2. UI-only stage fields (name, artifact_kind) ride inside `params` so
//    they survive the round-trip without forcing a backend schema change.
export function documentToCreateRequest(
  doc: WorkflowDocument,
  installations: GitHubAppInstallation[],
  workspaceId: string,
  activate: boolean,
): CreateWorkflowRequest {
  if (!workspaceId.trim()) {
    throw new ApiError("workspace_id is required", 400);
  }
  if (!isTriggerReady(doc.trigger)) {
    throw new ApiError(
      doc.trigger.kind_tag === "manual"
        ? "name the manual trigger before activating"
        : "pick an install, repo, and label before activating",
      400,
    );
  }

  // Manual triggers (#561) are repo-less: emit the `manual` variant with
  // just its button name — no `repo`, no install token-mint hint. The
  // backend's `trigger.repo` is optional (#545) and a Manual workflow
  // resolves repos workspace-scoped at runtime (#546/#556).
  if (doc.trigger.kind_tag === "manual") {
    return {
      workspace_id: workspaceId,
      name: doc.name.trim(),
      trigger: {
        kind: "manual",
        name: doc.trigger.manual_name.trim(),
      },
      stages: doc.stages.map(stageToCreateStage),
      active: activate,
    };
  }

  const install = installations.find((i) => i.id === doc.trigger.install_id);
  if (!install) {
    throw new ApiError(
      "selected GitHub install not found in this workspace",
      400,
    );
  }
  return {
    workspace_id: workspaceId,
    name: doc.name.trim(),
    // Structured trigger (ADR 0024 / spec #509). The GitHub-webhook leg is
    // unchanged from before #561 — `github_issue_webhook` with a `repo`.
    trigger: {
      kind: "github_issue_webhook",
      repo: `${doc.trigger.repo_owner}/${doc.trigger.repo_name}`,
      label: doc.trigger.label,
    },
    // Optional token-mint cache hint; resolved live through the project
    // layer at activation when absent.
    install_id: install.install_id,
    stages: doc.stages.map(stageToCreateStage),
    active: activate,
  };
}
