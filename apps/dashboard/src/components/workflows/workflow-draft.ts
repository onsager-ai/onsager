import {
  ApiError,
  stageToCreateStage,
  type CreateWorkflowRequest,
  type GitHubAppInstallation,
  type WorkflowGateKind,
  type WorkflowStage,
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

/**
 * One GitHub repository selected as a trigger source. Owner + name identify
 * the repo; `install_id` is the dashboard installation *record id*
 * (`GitHubAppInstallation.id`) the repo was discovered under — kept per-repo
 * because a workspace can have several installs and the token-mint hint is
 * resolved per install.
 */
export interface RepoRef {
  install_id: string;
  repo_owner: string;
  repo_name: string;
}

export interface WorkflowTriggerDraft {
  /**
   * Snake-case registry `kind_tag` for the selected trigger kind (e.g.
   * `'manual'`, `'cron'`). Drives `isTriggerReady`'s per-kind readiness
   * branch and which `TriggerKind` variant `documentToCreateRequest`
   * emits. Defaults to `'manual'` — the repo-less, always-ready kind (see
   * `emptyDocument`). `'github_issue_webhook'` is still a valid value
   * (existing drafts, the wire) but is hidden from the builder's kind
   * toggle for now.
   */
  kind_tag: string;
  /**
   * Canonical *primary* repo — the first of `repos`. Kept as flat fields
   * (not just `repos[0]`) so persisted drafts (localStorage, YAML, presets,
   * the create request) round-trip unchanged: the wire `TriggerKind` only
   * carries one `repo` until backend #566 lands per-workflow multi-repo
   * binding. Always mirror these from `repos[0]` via `setTriggerRepos`.
   */
  install_id: string;
  repo_owner: string;
  repo_name: string;
  /**
   * Full multi-repo selection (#566). Optional for back-compat with drafts
   * persisted before this field existed — read through `triggerRepos`, which
   * falls back to the flat primary fields. The extra repos beyond `repos[0]`
   * are frontend-ready but not yet honored by the backend; the create
   * request emits only the primary repo and the trigger sheet surfaces a
   * pending-#566 note when more than one is selected.
   */
  repos?: RepoRef[];
  label: string;
  /**
   * Cron schedule when `kind_tag === 'cron'` — a 5- or 6-field cron
   * expression. Empty for every other kind. Optional for back-compat with
   * drafts persisted before cron was authorable.
   */
  expression?: string;
  /**
   * Optional IANA timezone for the cron schedule (e.g. `'America/New_York'`).
   * `null`/absent = the scheduler's default (UTC).
   */
  timezone?: string | null;
}

/**
 * Normalized repo list for a trigger: prefers the explicit `repos` array,
 * falling back to the flat primary fields for drafts saved before `repos`
 * existed. Returns `[]` when no repo is set.
 */
export function triggerRepos(t: WorkflowTriggerDraft): RepoRef[] {
  if (t.repos && t.repos.length > 0) return t.repos;
  if (t.repo_owner && t.repo_name) {
    return [
      {
        install_id: t.install_id,
        repo_owner: t.repo_owner,
        repo_name: t.repo_name,
      },
    ];
  }
  return [];
}

/**
 * Set the repo selection and keep the canonical primary fields mirrored to
 * `repos[0]`, so every consumer that still reads the flat fields (create
 * request, presets, readiness, YAML) stays correct.
 */
export function setTriggerRepos(
  t: WorkflowTriggerDraft,
  repos: RepoRef[],
): WorkflowTriggerDraft {
  const primary = repos[0];
  return {
    ...t,
    repos,
    install_id: primary?.install_id ?? "",
    repo_owner: primary?.repo_owner ?? "",
    repo_name: primary?.repo_name ?? "",
  };
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
  name?: string,
): WorkflowStage {
  return {
    id: newStageId(),
    name: name ?? defaultStageName(gate),
    gate_kind: gate,
    config: {},
  };
}

export function defaultStageName(gate: WorkflowGateKind): string {
  switch (gate) {
    case "agent-session":
      return "Agent session";
    case "external-check":
      return "CI check";
    case "manual-approval":
      return "Manual approval";
  }
}

export function emptyDocument(): WorkflowDocument {
  return {
    name: "",
    trigger: {
      kind_tag: "manual",
      install_id: "",
      repo_owner: "",
      repo_name: "",
      repos: [],
      label: "",
      expression: "",
      timezone: null,
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
      makeStage("agent-session", "Spec → PR"),
      makeStage("external-check", "CI check"),
      makeStage("manual-approval", "Merge approval"),
    ],
  };
}

// Preset catalog — drives the picker shown at the top of the workflow
// builder. Each entry returns a fresh document; the trigger stays empty
// and is filled in via the TriggerEditor.
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
      stages: [makeStage("agent-session", "Agent session")],
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
        makeStage("external-check", "CI check"),
        makeStage("manual-approval", "Merge approval"),
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
        makeStage("external-check", "Merge wait"),
        makeStage("agent-session", "Deploy to staging"),
      ],
    }),
  },
];

export function isTriggerReady(t: WorkflowTriggerDraft): boolean {
  // Manual triggers (#561) are repo-less and carry no separate name — the
  // fire button derives its label from the workflow name — so a manual
  // trigger is always ready.
  if (t.kind_tag === "manual") {
    return true;
  }
  // Cron triggers need a schedule expression but no repo — runs resolve
  // repos workspace-scoped at fire time, same as manual.
  if (t.kind_tag === "cron") {
    return (t.expression ?? "").trim() !== "";
  }
  // The GitHub webhook leg stays exactly as it was (#545: webhook repo is
  // required, no regression) — hidden from the builder toggle for now, but
  // still honored for existing drafts.
  return (
    t.install_id.trim() !== "" &&
    t.repo_owner.trim() !== "" &&
    t.repo_name.trim() !== "" &&
    t.label.trim() !== ""
  );
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
// 2. The UI-only stage `name` rides inside `params` so it survives the
//    round-trip without forcing a backend schema change.
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
      "pick an install, repo, and label before activating",
      400,
    );
  }

  // Manual triggers (#561) are repo-less and carry no separate name: emit the
  // `manual` variant with the workflow name as its fire-button label — no
  // `repo`, no install token-mint hint. The backend's `trigger.repo` is
  // optional (#545) and a Manual workflow resolves repos workspace-scoped at
  // runtime (#546/#556).
  if (doc.trigger.kind_tag === "manual") {
    return {
      workspace_id: workspaceId,
      name: doc.name.trim(),
      trigger: {
        kind: "manual",
        name: doc.name.trim(),
      },
      stages: doc.stages.map(stageToCreateStage),
      active: activate,
    };
  }

  // Cron triggers (#238) are repo-less and time-based: emit the `cron`
  // variant with the schedule expression and optional timezone. Like manual,
  // they resolve repos workspace-scoped at fire time — no `repo`, no install
  // token-mint hint.
  if (doc.trigger.kind_tag === "cron") {
    const expression = (doc.trigger.expression ?? "").trim();
    const timezone = doc.trigger.timezone?.trim() || null;
    return {
      workspace_id: workspaceId,
      name: doc.name.trim(),
      trigger: {
        kind: "cron",
        expression,
        timezone,
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
  // Multi-repo binding is authored in the UI (`trigger.repos`) but the wire
  // `TriggerKind::github_issue_webhook` carries a single `repo` until backend
  // #566 lands. Emit the primary repo (`repos[0]`, mirrored to the flat
  // fields) and warn so extra repos never silently vanish. When #566 ships a
  // repo array, widen the trigger shape here and drop this guard.
  const repos = triggerRepos(doc.trigger);
  if (repos.length > 1) {
    console.warn(
      `[workflow-draft] ${repos.length} repos selected but the backend ` +
        `binds one per workflow (#566); creating with ${doc.trigger.repo_owner}/${doc.trigger.repo_name} only.`,
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
