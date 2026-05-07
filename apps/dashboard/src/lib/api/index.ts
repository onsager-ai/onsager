// Re-export client primitives
export { ApiError } from './client';

// Re-export all types
export type {
  Node,
  Session,
  TaskRequest,
  User,
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
  GovernanceStats,
  GovernanceRule,
  IsingInsightEmittedEvent,
  RuleProposal,
  TokenUsage,
  SessionSpend,
  SpineEvent,
  SpineArtifact,
  ProjectIssueRow,
  ProjectIssueDetail,
  ProjectIssueDetailResponse,
  ProjectPullRow,
  ProjectLiveListResponse,
  BackfillRequestBody,
  ReplayIssueTriggerRequest,
  ReplayMatch,
  ReplayIssueTriggerResponse,
  BackfillReport,
  ArtifactDetail,
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
  StageRunStatus,
  WorkflowRunStage,
  WorkflowRun,
  GitHubLabel,
  RegisterArtifactRequest,
  ArtifactActionRequest,
  OverrideGateRequestBody,
  ArtifactActionResponse,
} from './types';

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
};
