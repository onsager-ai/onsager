import { request } from './client';
import type {
  Workspace,
  WorkspaceMember,
  GitHubAppInstallation,
  GitHubAccountType,
  Project,
  AccessibleRepo,
  GitHubLabel,
  WorkspaceDeliveryHealthResponse,
} from './types';

export const workspaces = {
  // Workspaces (issue #59; renamed from `/tenants` per #163). The dashboard
  // hits the new path directly. Wire envelope is `workspaces`/`workspace`
  // post-rename.
  listWorkspaces: () => request<{ workspaces: Workspace[] }>('/workspaces'),
  createWorkspace: (body: { slug: string; name: string }) =>
    request<{ workspace: Workspace }>('/workspaces', {
      method: 'POST',
      body: JSON.stringify(body),
    }),
  getWorkspace: (id: string) =>
    request<{ workspace: Workspace }>(`/workspaces/${encodeURIComponent(id)}`),
  listWorkspaceMembers: (id: string) =>
    request<{ members: WorkspaceMember[] }>(
      `/workspaces/${encodeURIComponent(id)}/members`,
    ),
  listWorkspaceInstallations: (id: string) =>
    request<{ installations: GitHubAppInstallation[] }>(
      `/workspaces/${encodeURIComponent(id)}/github-installations`,
    ),
  registerWorkspaceInstallation: (
    id: string,
    body: {
      install_id: number;
      account_login: string;
      account_type: GitHubAccountType;
      webhook_secret?: string;
    },
  ) =>
    request<{ installation: GitHubAppInstallation }>(
      `/workspaces/${encodeURIComponent(id)}/github-installations`,
      { method: 'POST', body: JSON.stringify(body) },
    ),
  deleteWorkspaceInstallation: (workspaceId: string, installId: string) =>
    request<{ ok: boolean }>(
      `/workspaces/${encodeURIComponent(workspaceId)}/github-installations/${encodeURIComponent(installId)}`,
      { method: 'DELETE' },
    ),
  listWorkspaceProjects: (id: string) =>
    request<{ projects: Project[] }>(
      `/workspaces/${encodeURIComponent(id)}/projects`,
    ),
  addWorkspaceProject: (
    id: string,
    body: {
      github_app_installation_id: string;
      repo_owner: string;
      repo_name: string;
      default_branch?: string;
    },
  ) =>
    request<{ project: Project }>(
      `/workspaces/${encodeURIComponent(id)}/projects`,
      { method: 'POST', body: JSON.stringify(body) },
    ),
  listAllProjects: () => request<{ projects: Project[] }>('/projects'),
  deleteProject: (id: string) =>
    request<{ ok: boolean }>(`/projects/${encodeURIComponent(id)}`, {
      method: 'DELETE',
    }),
  // GitHub App install flow + accessible-repos picker (closes the last
  // Phase 0 items from #59: OAuth callback and the repo dropdown).
  getGitHubAppConfig: () =>
    request<{ enabled: boolean; slug?: string | null }>('/github-app/config'),
  listInstallationRepos: (workspaceId: string, installId: string) =>
    request<{ repos: AccessibleRepo[] }>(
      `/workspaces/${encodeURIComponent(workspaceId)}/github-installations/${encodeURIComponent(installId)}/accessible-repos`,
    ),
  // GitHub labels for a workspace install + repo. Used by the trigger card
  // combobox so the user selects from existing labels (with an inline
  // create-new affordance) instead of free-texting.
  listRepoLabels: (workspaceId: string, installId: string, owner: string, repo: string) =>
    request<{ labels: GitHubLabel[] }>(
      `/workspaces/${encodeURIComponent(workspaceId)}/github-installations/${encodeURIComponent(installId)}/repos/${encodeURIComponent(owner)}/${encodeURIComponent(repo)}/labels`,
    ),
  // Webhook delivery health for every installation in the workspace —
  // last K=30 deliveries summarised per install (spec #120 item 3).
  // Powers the workflow card's "webhook deliveries failing" warning.
  getWorkspaceWebhookDeliveriesHealth: (workspaceId: string) =>
    request<WorkspaceDeliveryHealthResponse>(
      `/workspaces/${encodeURIComponent(workspaceId)}/webhook-deliveries-health`,
    ),
};
