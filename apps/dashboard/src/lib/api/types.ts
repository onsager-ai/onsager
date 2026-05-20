// Wire-shape types for the dashboard API. Per spec #298, Rust serde structs
// in `crates/onsager-portal/` are the single source of truth — `ts-rs`
// emits the canonical bindings into `./generated/`. This file re-exports
// the generated types alongside the residual hand-written types that
// haven't been derived yet (workflow cascade, run/stage typing, synodic
// proxy types). Once those land (umbrella #298 Phase 3) this file collapses
// to a pure re-export barrel.

// ── Generated (Rust is SSOT) ─────────────────────────────────────────────

export type { AccessibleRepo } from './generated/AccessibleRepo';
export type { ArtifactHorizontalLineageEntry } from './generated/ArtifactHorizontalLineageEntry';
export type { ArtifactLineageEntry } from './generated/ArtifactLineageEntry';
export type { ArtifactVersion } from './generated/ArtifactVersion';
export type { BackfillReport } from './generated/BackfillReport';
export type { BackfillRequestBody } from './generated/BackfillRequestBody';
export type { CreatePatResponse } from './generated/CreatePatResponse';
export type { Credential } from './generated/Credential';
export type { GitHubAccountType } from './generated/GitHubAccountType';
export type { GitHubAppInstallation } from './generated/GitHubAppInstallation';
export type { InstallationDeliveryHealth } from './generated/InstallationDeliveryHealth';
export type { Node } from './generated/Node';
export type { NodeStatus } from './generated/NodeStatus';
export type { Pat } from './generated/Pat';
export type { Project } from './generated/Project';
export type { ProjectIssueDetail } from './generated/ProjectIssueDetail';
export type { ProjectIssueDetailResponse } from './generated/ProjectIssueDetailResponse';
export type { ProjectPullRow } from './generated/ProjectPullRow';
export type { ReplayIssueTriggerRequest } from './generated/ReplayIssueTriggerRequest';
export type { ReplayIssueTriggerResponse } from './generated/ReplayIssueTriggerResponse';
export type { ReplayMatch } from './generated/ReplayMatch';
export type { Session } from './generated/Session';
export type { SessionKind } from './generated/SessionKind';
export type { SessionState } from './generated/SessionState';
export type { SpineArtifact } from './generated/SpineArtifact';
export type { SpineEvent } from './generated/SpineEvent';
export type { TaskRequest } from './generated/TaskRequest';
export type { TokenUsage } from './generated/TokenUsage';
export type { WorkflowGateKind } from './generated/WorkflowGateKind';
export type { Workspace } from './generated/Workspace';
export type { WorkspaceDeliveryHealthResponse } from './generated/WorkspaceDeliveryHealthResponse';
export type { WorkspaceMember } from './generated/WorkspaceMember';

// Local imports for the residual hand-written wrappers below. `TokenUsage`
// is consumed by the client-side-only `SessionSpend` row.
import type { TokenUsage } from './generated/TokenUsage';

// ── Hand-written (pending derive) ────────────────────────────────────────

// User identity returned by `/api/auth/me`. The portal-side `User` struct
// in `auth_db.rs` carries extra audit fields (`github_id`, `created_at`,
// `updated_at`) that don't appear on the wire. Deriving directly would
// require typing the `me` handler response — tracked as a Phase 2
// follow-up.
export interface User {
  id: string;
  github_login: string;
  github_name: string | null;
  github_avatar_url: string | null;
}

// Denormalized session-completion spend row (issue #39). Constructed
// client-side from `stiglab.session_completed` events — there is no
// portal-side endpoint that produces this shape, so it stays hand-written.
export interface SessionSpend {
  id: number;
  created_at: string;
  session_id: string;
  artifact_id: string | null;
  duration_ms: number;
  token_usage: TokenUsage | null;
}

// Governance proxy types (`/api/governance/*`) — portal forwards bytes to
// synodic without parsing. Deriving these would require teaching ts-rs
// about synodic structs (out of scope for this PR; tracked under #298).
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

// Ising insight surface (issue #36). Persisted as a spine event row
// (`ising.insight_emitted`); the dashboard reads via the events endpoint.
// Same reason as the governance types — derivation lives outside portal.
export interface IsingInsightEmittedEvent {
  id: number;
  created_at: string;
  signal_kind: string;
  subject_ref: string;
  confidence: number;
  evidence: { event_id: number; event_type: string }[];
}

// Ising rule proposal queue (issue #36 Step 2). Proxied through Synodic.
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

// Generic envelope shared by every `/api/projects/:id/{issues,pulls}` list
// endpoint (#170 fail-open). Stays hand-written because ts-rs has no
// generic-output support; the two instantiations are concrete elsewhere.
export interface ProjectLiveListResponse<T> {
  issues?: T[];
  pulls?: T[];
  /// `rate_limited` / `github_unreachable` per #170 fail-open. Dashboard
  /// renders the artifact skeleton's `last_observed_at` placeholder.
  error?: string;
}

// Workflows (issue #82). The full CRUD surface is hand-written because
// `WorkflowTrigger` cascades into `onsager-spine::TriggerKind` whose
// 6+ variant tree (`PullRequestClosedPredicate`, `DelayAnchor`,
// `JsonFilter`, …) needs ts-rs derives across the spine crate first.
// Tracked as #298 sub-issue B.
//
// Artifact kinds are registry-backed as of #102. `WorkflowArtifactKind` is
// a string so any kind registered server-side is representable on the wire.
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

// Shape returned by `GET /api/workflow/kinds` (issue #102).
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

// Workflow CRUD wrapper shape used by the dashboard's workflow client.
// The generated `WorkflowStage` describes the spine row; this richer
// wrapper carries the UI-side `name` / `artifact_kind` / `config`. Phase B
// (#298 sub-issue B) collapses these.
export interface WorkflowStage {
  id: string;
  name: string;
  gate_kind: import('./generated/WorkflowGateKind').WorkflowGateKind;
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
  gate_kind: import('./generated/WorkflowGateKind').WorkflowGateKind;
  params: Record<string, unknown>;
}

/** A session linked back to a run via `sessions.artifact_id` (spec #303). */
export interface RunLinkedSession {
  id: string;
  state: import('./generated/SessionState').SessionState;
  node_id: string;
  created_at: string;
  updated_at: string;
}

/** Combined response shape for `GET /api/runs/:id` (spec #303). */
export interface RunDetail {
  run: import('./generated/WorkflowRun').WorkflowRun;
  workflow: Workflow;
  stages: WorkflowStage[];
  sessions: RunLinkedSession[];
}

// GitHub label mirror used by the workflow-builder UI. Hand-written
// because the same shape is produced by stiglab's project label proxy,
// not portal.
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
