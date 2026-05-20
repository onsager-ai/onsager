// Re-export client primitives
export { ApiError } from './client';

// Re-export all types
export type {
  Node,
  Session,
  TaskRequest,
  MeUser,
  MeResponse,
  MeVia,
  SessionKind,
  Credential,
  Pat,
  CreatePatResponse,
  Workspace,
  WorkspaceMember,
  GitHubAccountType,
  GitHubAppInstallation,
  Project,
  AccessibleRepo,
  GovernanceEvent,
  TokenUsage,
  SpineEvent,
  SpineArtifact,
  ProjectIssueDetail,
  ProjectIssueDetailResponse,
  ProjectPullRow,
  BackfillRequestBody,
  ReplayIssueTriggerRequest,
  ReplayMatch,
  ReplayIssueTriggerResponse,
  BackfillReport,
  ArtifactVersion,
  ArtifactLineageEntry,
  ArtifactHorizontalLineageEntry,
  WorkflowArtifactKind,
  WorkflowTrigger,
  WorkflowGateKind,
  WorkflowMergeRule,
  JsonValue,
  WorkflowKindInfo,
  EventSubsystem,
  EventManifestEntry,
  TriggerCategory,
  TriggerUiKind,
  TriggerManifestEntry,
  WorkflowStage,
  WorkflowStatus,
  Workflow,
  CreateWorkflowRequest,
  CreateWorkflowStage,
  RunDetail,
  RunLinkedSession,
  GitHubLabel,
  ArtifactActionRequest,
  OverrideGateRequestBody,
  ArtifactActionResponse,
  InstallationDeliveryHealth,
  WorkspaceDeliveryHealthResponse,
} from './types';

// Generated from Rust serde structs (spec #298 Phase 2 / #435).
export type { ArtifactDetail } from './generated/ArtifactDetail';
export type { WorkflowRun } from './generated/WorkflowRun';
export type { WorkflowRunStage } from './generated/WorkflowRunStage';
export type { StageRunStatus } from './generated/StageRunStatus';

// `SessionSpend` is co-located with its derivation (spec #441) — there is
// no portal endpoint that produces this shape.
export type { SessionSpend } from './sessions';

// Re-export workflow helpers
export { normalizeWorkflowArtifactKind, stageToCreateStage } from './workflows';

// Compose the unified `api` object that all existing callers use
import { sessions } from './sessions';
import { credentials } from './credentials';
import { pats } from './pats';
import { workspaces } from './workspaces';
import { governance } from './governance';
import { spine } from './spine';
import { artifacts } from './artifacts';
import { workflows } from './workflows';
import { issues } from './issues';
import { pulls } from './pulls';
import { registry } from './registry';
import { activation } from './activation';

export const api = {
  ...sessions,
  ...credentials,
  ...pats,
  ...workspaces,
  ...governance,
  ...spine,
  ...artifacts,
  ...workflows,
  ...issues,
  ...pulls,
  ...registry,
  ...activation,
};
