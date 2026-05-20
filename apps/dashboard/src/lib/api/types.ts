export interface Node {
  id: string;
  name: string;
  hostname: string;
  status: 'online' | 'offline' | 'draining';
  max_sessions: number;
  active_sessions: number;
  last_heartbeat: string;
  registered_at: string;
}

export interface Session {
  id: string;
  task_id: string;
  node_id: string;
  state: 'pending' | 'dispatched' | 'running' | 'waiting_input' | 'done' | 'failed';
  prompt: string;
  output: string | null;
  working_dir: string | null;
  artifact_id: string | null;
  created_at: string;
  updated_at: string;
}

export interface TaskRequest {
  prompt: string;
  node_id?: string;
  working_dir?: string;
  allowed_tools?: string[];
  max_turns?: number;
  project_id?: string;
}

export interface User {
  id: string;
  github_login: string;
  github_name: string | null;
  github_avatar_url: string | null;
}

/**
 * How the active session was minted (issue #193). `"github"` for a real
 * OAuth session; `"dev"` for a `${USER}@local` session minted by the
 * `/api/auth/dev-login` flow available only in debug builds.
 */
export type SessionKind = 'github' | 'dev';

export interface Credential {
  name: string;
  created_at: string;
  updated_at: string;
}

// Personal Access Tokens (issue #143). The full token value is only ever
// returned by `createPat`; subsequent `listPats` calls expose prefix +
// metadata only. `workspace_id` pins the PAT to a workspace.
export interface Pat {
  id: string;
  name: string;
  workspace_id: string;
  token_prefix: string;
  expires_at: string | null;
  last_used_at: string | null;
  last_used_ip: string | null;
  last_used_user_agent: string | null;
  created_at: string;
  revoked_at: string | null;
}

export interface CreatePatResponse {
  pat: Pat;
  /// Returned exactly once on creation. After this response, the only way
  /// to recover access is to mint a new token.
  token: string;
}

// Workspace / membership / GitHub App installation / project types,
// issue #59 (Phase 0); renamed from `tenant` per #163.
export interface Workspace {
  id: string;
  slug: string;
  name: string;
  created_by: string;
  created_at: string;
}

export interface WorkspaceMember {
  workspace_id: string;
  user_id: string;
  joined_at: string;
  github_login: string | null;
  github_name: string | null;
  github_avatar_url: string | null;
}

export type GitHubAccountType = 'user' | 'organization';

export interface GitHubAppInstallation {
  id: string;
  workspace_id: string;
  install_id: number;
  account_login: string;
  account_type: GitHubAccountType;
  created_at: string;
}

// Webhook delivery health for one installation (spec #120 item 3).
// `checked` may be 0 when GitHub hasn't routed any deliveries to this
// installation in the recent window — distinguishable from "App not
// configured on this server", which manifests as the whole response
// having `window: 0` and `checked: 0` for every installation.
export interface InstallationDeliveryHealth {
  install_id: number;
  checked: number;
  non_2xx: number;
  last_delivered_at: string | null;
  last_non_2xx_at: string | null;
  last_non_2xx_status_code: number | null;
}

export interface WorkspaceDeliveryHealthResponse {
  installations: InstallationDeliveryHealth[];
  window: number;
}

export interface Project {
  id: string;
  workspace_id: string;
  github_app_installation_id: string;
  repo_owner: string;
  repo_name: string;
  default_branch: string;
  created_at: string;
}

export interface AccessibleRepo {
  owner: string;
  name: string;
  default_branch: string | null;
  private: boolean;
}

// Governance types (proxied to synodic via /api/governance/*)
export interface GovernanceEvent {
  id: string;
  event_type: string;
  title: string;
  severity: string;
  source: string;
  metadata: Record<string, unknown>;
  resolved: boolean;
  resolution_notes: string | null;
  created_at: string;
  resolved_at: string | null;
}

export interface GovernanceStats {
  total: number;
  unresolved: number;
  by_type: Record<string, number>;
  by_severity: Record<string, number>;
}

export interface GovernanceRule {
  name: string;
  description: string;
  pattern: string;
  event_type: string;
  severity: string;
  enabled: boolean;
}

// Ising insight surface (issue #36). The emitter writes events to the spine
// as `ising.insight_emitted`; the dashboard reads them back via the spine
// events endpoint and presents the structured fields directly.
export interface IsingInsightEmittedEvent {
  id: number;
  created_at: string;
  signal_kind: string;
  subject_ref: string;
  confidence: number;
  evidence: { event_id: number; event_type: string }[];
}

// Ising rule proposal queue (issue #36 Step 2). Proxied through Synodic —
// each row corresponds to one `ising.rule_proposed` event the listener
// ingested and is awaiting human (or supervisor-agent) resolution.
export interface RuleProposal {
  id: string;
  insight_id: string;
  signal_kind: string;
  subject_ref: string;
  proposed_action: Record<string, unknown>;
  class: 'safe_auto' | 'review_required';
  rationale: string;
  confidence: number;
  status: 'pending' | 'approved' | 'rejected';
  resolution_notes: string | null;
  created_at: string;
  resolved_at: string | null;
}

// Token usage carried by `stiglab.session_completed` events (issue #39).
export interface TokenUsage {
  input_tokens: number;
  output_tokens: number;
  cache_read_tokens?: number;
  cache_write_tokens?: number;
  model?: string;
}

// Denormalized session-completion spend row (issue #39), projected from
// `stiglab.session_completed` spine events.
export interface SessionSpend {
  id: number;
  created_at: string;
  session_id: string;
  artifact_id: string | null;
  duration_ms: number;
  token_usage: TokenUsage | null;
}

// Spine types (direct from stiglab)
export interface SpineEvent {
  id: number;
  stream_id: string;
  stream_type: string;
  event_type: string;
  data: Record<string, unknown>;
  actor: string;
  created_at: string;
}

export interface SpineArtifact {
  id: string;
  kind: string;
  /// Reference-only artifacts (`kind in ['github_issue', 'pull_request']`,
  /// per spec #170) carry NULL here — the title is GitHub-authored and is
  /// served by the live-hydration proxy at `/api/projects/:id/{issues,pulls}`.
  /// Older PR rows materialized before the migration retain their stale
  /// titles as best-effort fallback display.
  name: string | null;
  /// Reference-only artifacts carry NULL — see `name` for rationale.
  owner: string | null;
  state: string;
  current_version: number;
  consumers?: string[];
  /// Stable handle for joining skeleton rows with live proxy responses
  /// (`github:project:{project_id}:{issue,pr}:{number}`).
  external_ref?: string | null;
  created_at: string;
  updated_at: string;
  /// Last webhook touch — drives the "last seen N min ago" placeholder
  /// when the proxy is rate-limited (#170 fail-open).
  last_observed_at?: string | null;
}

/// Live-hydrated GitHub issue from `GET /api/projects/:id/issues`. Joined
/// with `SpineArtifact` rows (kind=`github_issue`) on `external_ref` to
/// build the dashboard inbox view (#168).
export interface ProjectIssueRow {
  number: number;
  title: string;
  state: string;
  html_url: string;
  author: string | null;
  labels: string[];
  comments: number;
  updated_at: string;
}

/// Detail-shape counterpart to `ProjectIssueRow` (#205). Adds the
/// fields the list endpoint omits — body, assignees, milestone, and
/// the created/closed timestamps. Hits the same proxy cache as the
/// list endpoint and uses the same fail-open envelope: `issue` is
/// null and `error` is set when the upstream is rate-limited or
/// unreachable.
export interface ProjectIssueDetail {
  number: number;
  title: string;
  state: string;
  html_url: string;
  author: string | null;
  labels: string[];
  assignees: string[];
  comments: number;
  body: string | null;
  milestone: { title: string; state: string } | null;
  created_at: string | null;
  updated_at: string;
  closed_at: string | null;
}

export interface ProjectIssueDetailResponse {
  issue: ProjectIssueDetail | null;
  error?: string;
}

export interface ProjectPullRow {
  number: number;
  title: string;
  state: string;
  html_url: string;
  author: string | null;
  labels: string[];
  draft: boolean;
  merged: boolean;
  updated_at: string;
}

export interface ProjectLiveListResponse<T> {
  issues?: T[];
  pulls?: T[];
  /// `rate_limited` / `github_unreachable` per #170 fail-open. Dashboard
  /// renders the artifact skeleton's `last_observed_at` placeholder.
  error?: string;
}

export interface BackfillRequestBody {
  cap?: number;
  strategy?: 'recent' | 'active' | 'prioritized';
  state?: 'open' | 'closed' | 'all';
}

/// Manual replay of a `workflow.trigger_fired` event for one issue
/// (spec #203). Active counterpart to the passive
/// `issues.labeled` webhook path — used for debugging when no e2e
/// workflow run has fired yet.
export interface ReplayIssueTriggerRequest {
  /// `true` (default on the server) returns the matched workflow list
  /// without emitting; `false` emits one event per match.
  dry_run?: boolean;
}

export interface ReplayMatch {
  workflow_id: string;
  workflow_name: string;
  label: string;
}

export interface ReplayIssueTriggerResponse {
  project_id: string;
  issue_number: number;
  dry_run: boolean;
  matches: ReplayMatch[];
  /// Spine event IDs for the emitted events. Empty in dry-run mode and
  /// when there were zero matches.
  event_ids: number[];
}

export interface BackfillReport {
  project_id: string;
  repo: string;
  cap: number;
  issues_ingested: number;
  pulls_ingested: number;
  skipped: number;
}

export interface ArtifactDetail extends SpineArtifact {
  created_by: string;
  versions: ArtifactVersion[];
  vertical_lineage: ArtifactLineageEntry[];
  horizontal_lineage?: ArtifactHorizontalLineageEntry[];
  related_events?: SpineEvent[];
}

export interface ArtifactVersion {
  version: number;
  content_ref_uri: string;
  content_ref_checksum: string | null;
  change_summary: string;
  created_by_session: string;
  parent_version: number | null;
  created_at: string;
}

export interface ArtifactLineageEntry {
  version: number;
  session_id: string;
  recorded_at: string;
}

// Horizontal lineage — cross-artifact dependency edges, e.g. a PR's
// `closes_issue` link back to the spec issue it implements.
export interface ArtifactHorizontalLineageEntry {
  source_artifact_id: string;
  source_version: number;
  role: string;
  recorded_at: string;
}

// Workflows (issue #82). A workflow is a trigger + an ordered list of stage
// cards. Triggers fire on external events (currently GitHub issue labels);
// each stage runs a gate that moves artifacts along — agent sessions,
// external checks, governance verdicts, or manual approvals.
//
// The CRUD API is delivered by a parallel sibling sub-issue of #79; this
// client is the typed surface the dashboard UI talks to.
//
// Artifact kinds are registry-backed as of #102. `WorkflowArtifactKind` is
// a string so any kind registered server-side is representable on the wire;
// the static-fallback list in `workflow-meta.ts` is what the UI renders
// when the runtime fetch fails (offline / dev without stiglab).
export type WorkflowArtifactKind = string;

export interface WorkflowTrigger {
  kind: 'github-label';
  install_id: string;
  repo_owner: string;
  repo_name: string;
  label: string;
  /// Snake-case wire `kind_tag` from the registry manifest (e.g.
  /// `'github_issue_webhook'`, `'manual'`). Set on every workflow,
  /// regardless of the UI-side discriminant; the `<RunNowButton>`
  /// keys off this rather than the legacy `kind` field so it can
  /// render for `'manual'` workflows without a UI-side variant.
  kind_tag: string;
  /// Manual-trigger name when `kind_tag === 'manual'`. Empty for
  /// other kinds.
  manual_name?: string;
}

export type WorkflowGateKind =
  | 'agent-session'
  | 'external-check'
  | 'governance'
  | 'manual-approval';

// Shape returned by `GET /api/workflow/kinds` (issue #102). The registry
// owns the canonical list; the dashboard's hardcoded set in
// `workflow-meta.ts` is only a fallback for offline/dev.
export type WorkflowMergeRule = 'overwrite' | 'merge_by_key' | 'append' | 'deep_merge';

// `intrinsic_schema` arrives as a `serde_json::Value`, which is any JSON
// value — including `null`, arrays, and primitives. Modelling it as
// `JsonValue` keeps the wire shape honest so consumers can't assume
// "always an object".
export type JsonValue =
  | string
  | number
  | boolean
  | null
  | { [key: string]: JsonValue }
  | JsonValue[];

export interface WorkflowKindInfo {
  id: string;
  description: string;
  merge_rule: WorkflowMergeRule;
  external_kind?: string;
  aliases: string[];
  intrinsic_schema: JsonValue;
}

// Wire shape of `GET /api/registry/events` — one row of the event-type
// registry manifest (spec #131 Lever E / #150). Mirrors
// `onsager_registry::EventDefinition`; keep field names + the
// `EventSubsystem` union in sync by hand when the Rust struct changes.
export type EventSubsystem = 'forge' | 'stiglab' | 'synodic' | 'ising' | 'portal';

export interface EventManifestEntry {
  kind: string;
  schema_version: number;
  producers: EventSubsystem[];
  consumers: EventSubsystem[];
  /**
   * True when no subsystem consumer is expected — the event is read by a
   * non-subsystem concern (dashboard timeline, audit trail). Paired with a
   * non-empty `reason`. Per spec #272, replaces the prior `audit_only`
   * field.
   */
  diagnostic_only: boolean;
  /**
   * Why this row is diagnostic-only (e.g. "rendered in dashboard event
   * timeline"). Required when `diagnostic_only` is true; null otherwise.
   */
  reason: string | null;
  description: string;
}

// Wire shape of `GET /api/registry/triggers` (spec #237). One row per
// `onsager_spine::TriggerKind` variant. Mirrors
// `onsager_registry::TriggerDefinition`; keep in sync with the Rust
// struct.
export type TriggerCategory = 'event' | 'schedule' | 'request' | 'manual';
export type TriggerUiKind =
  | 'webhook'
  | 'github_pull_request_closed'
  | 'github_workflow_run_completed'
  | 'telegram_webhook'
  | 'cron'
  | 'delay'
  | 'interval'
  | 'spine_event'
  | 'pg_notify'
  | 'outbox'
  | 'manual'
  | 'replay';

export interface TriggerManifestEntry {
  kind_tag: string;
  producer: EventSubsystem;
  category: TriggerCategory;
  ui_kind: TriggerUiKind;
  description: string;
}

export interface WorkflowStage {
  id: string;
  name: string;
  gate_kind: WorkflowGateKind;
  artifact_kind: WorkflowArtifactKind;
  config: Record<string, unknown>;
}

export type WorkflowStatus = 'draft' | 'active' | 'paused' | 'archived';

export interface Workflow {
  id: string;
  workspace_id: string;
  name: string;
  preset?: string | null;
  status: WorkflowStatus;
  trigger: WorkflowTrigger;
  stages: WorkflowStage[];
  created_at: string;
  updated_at: string;
}

// Wire contract for workflow CRUD. Matches stiglab's `CreateWorkflowBody`
// / `validate_create_body` exactly — flat trigger fields, numeric GitHub
// install id, snake_case `active`. The `trigger_kind` is the registry's
// snake-case `kind_tag` (e.g. `'github_issue_webhook'`) — fetched at
// runtime from `/api/registry/triggers` (spec #237). Construct with
// `documentToCreateRequest` from the UI draft + installations list so the
// numeric id is resolved from the workspace installation record id the
// draft carries.
export interface CreateWorkflowRequest {
  workspace_id: string;
  name: string;
  trigger_kind: string;
  repo_owner: string;
  repo_name: string;
  trigger_label: string;
  install_id: number;
  preset_id?: string;
  stages?: CreateWorkflowStage[];
  active: boolean;
}

export interface CreateWorkflowStage {
  gate_kind: WorkflowGateKind;
  params: Record<string, unknown>;
}

export type StageRunStatus = 'pending' | 'blocked' | 'passed' | 'failed';

export interface WorkflowRunStage {
  stage_id: string;
  status: StageRunStatus;
  updated_at: string;
}

export interface WorkflowRun {
  id: string;
  workflow_id: string;
  artifact_id: string | null;
  status: StageRunStatus;
  stages: WorkflowRunStage[];
  started_at: string;
  updated_at: string;
}

/** A session linked back to a run via `sessions.artifact_id` (spec #303). */
export interface RunLinkedSession {
  id: string;
  state: Session['state'];
  node_id: string;
  created_at: string;
  updated_at: string;
}

/** Combined response shape for `GET /api/runs/:id` (spec #303). */
export interface RunDetail {
  run: WorkflowRun;
  workflow: Workflow;
  stages: WorkflowStage[];
  sessions: RunLinkedSession[];
}

export interface GitHubLabel {
  name: string;
  color: string | null;
  description: string | null;
}

export interface ArtifactActionRequest {
  reason?: string;
  actor?: string;
}

export interface OverrideGateRequestBody extends ArtifactActionRequest {
  verdict?: 'allow' | 'deny';
}

export interface ArtifactActionResponse {
  artifact_id: string;
  action: string;
  verdict?: string;
  reason?: string;
  escalation_id?: string;
}
