import { useEffect, useState } from "react"
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query"
import { api, type Workspace, type GitHubAppInstallation, type Project } from "@/lib/api"
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/card"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { Building2, GitBranch, Plus, Trash2 } from "lucide-react"

/**
 * Settings card that lists the user's Workspaces and the nested
 * Members / GitHub installations / Projects for each.
 *
 * v1 is intentionally minimal (issue #59): no roles, no invites, no
 * cascades, no auto-mirror. GitHub App installations are registered
 * manually here — the full OAuth callback flow lands in a follow-up.
 */
export function WorkspacesCard() {
  const [creating, setCreating] = useState(false)
  const [newSlug, setNewSlug] = useState("")
  const [newName, setNewName] = useState("")
  const [createError, setCreateError] = useState<string | null>(null)
  const queryClient = useQueryClient()

  const { data: wsData } = useQuery({
    queryKey: ["workspaces"],
    queryFn: api.listWorkspaces,
  })

  const createMutation = useMutation({
    mutationFn: (body: { slug: string; name: string }) => api.createWorkspace(body),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["workspaces"] })
      setCreating(false)
      setNewSlug("")
      setNewName("")
      setCreateError(null)
    },
    onError: (err) => {
      setCreateError(err instanceof Error ? err.message : "Failed to create workspace")
    },
  })

  const workspaces = wsData?.tenants ?? []

  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2 text-base md:text-lg">
          <Building2 className="h-4 w-4" />
          Workspaces
        </CardTitle>
        <CardDescription>
          A workspace owns GitHub App installations and projects. Projects are
          opt-in per repo — installing the App on an organization does not
          auto-mirror its repositories.
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        {workspaces.length === 0 && !creating && (
          <p className="text-sm text-muted-foreground">
            You have no workspaces yet. Create one to onboard GitHub projects.
          </p>
        )}

        {workspaces.map((ws) => (
          <WorkspaceRow key={ws.id} workspace={ws} />
        ))}

        {creating ? (
          <form
            onSubmit={(e) => {
              e.preventDefault()
              if (createMutation.isPending) return
              if (newSlug && newName)
                createMutation.mutate({ slug: newSlug, name: newName })
            }}
            className="space-y-2 rounded-md border border-dashed p-3"
          >
            <p className="text-sm font-medium">New workspace</p>
            <div className="grid grid-cols-1 gap-2 sm:grid-cols-[auto_1fr]">
              <Input
                placeholder="slug (e.g. acme)"
                value={newSlug}
                onChange={(e) => setNewSlug(e.target.value.toLowerCase())}
                className="sm:w-48"
              />
              <Input
                placeholder="Display name"
                value={newName}
                onChange={(e) => setNewName(e.target.value)}
              />
            </div>
            {createError && (
              <p className="text-xs text-destructive">{createError}</p>
            )}
            <div className="flex items-center gap-2">
              <Button
                size="sm"
                type="submit"
                disabled={!newSlug || !newName || createMutation.isPending}
              >
                Create
              </Button>
              <Button
                size="sm"
                type="button"
                variant="outline"
                onClick={() => {
                  setCreating(false)
                  setNewSlug("")
                  setNewName("")
                  setCreateError(null)
                }}
              >
                Cancel
              </Button>
            </div>
          </form>
        ) : (
          <Button
            size="sm"
            variant="outline"
            onClick={() => {
              setCreating(true)
              setCreateError(null)
            }}
          >
            <Plus className="mr-1 h-3 w-3" />
            New workspace
          </Button>
        )}
      </CardContent>
    </Card>
  )
}

function WorkspaceRow({ workspace }: { workspace: Workspace }) {
  const { data: membersData } = useQuery({
    queryKey: ["workspace-members", workspace.id],
    queryFn: () => api.listWorkspaceMembers(workspace.id),
  })
  const { data: installsData } = useQuery({
    queryKey: ["workspace-installations", workspace.id],
    queryFn: () => api.listWorkspaceInstallations(workspace.id),
  })
  const { data: projectsData } = useQuery({
    queryKey: ["workspace-projects", workspace.id],
    queryFn: () => api.listWorkspaceProjects(workspace.id),
  })

  const members = membersData?.members ?? []
  const installations = installsData?.installations ?? []
  const projects = projectsData?.projects ?? []

  return (
    <div className="space-y-3 rounded-md border p-3">
      <div>
        <p className="font-medium">{workspace.name}</p>
        <p className="text-xs text-muted-foreground">
          Slug: <span className="font-mono">{workspace.slug}</span> · Created{" "}
          {new Date(workspace.created_at).toLocaleDateString()}
        </p>
      </div>

      {/* Members (read-only in v1) */}
      <div className="space-y-1">
        <p className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
          Members ({members.length})
        </p>
        <p className="text-xs text-muted-foreground">
          {members.map((m) => m.user_id).join(", ") || "—"}
        </p>
      </div>

      {/* GitHub installations */}
      <InstallationsSection
        workspaceId={workspace.id}
        installations={installations}
      />

      {/* Projects */}
      <ProjectsSection
        workspaceId={workspace.id}
        installations={installations}
        projects={projects}
      />
    </div>
  )
}

function InstallationsSection({
  workspaceId,
  installations,
}: {
  workspaceId: string
  installations: GitHubAppInstallation[]
}) {
  const [adding, setAdding] = useState(false)
  const [installId, setInstallId] = useState("")
  const [accountLogin, setAccountLogin] = useState("")
  const [accountType, setAccountType] =
    useState<"user" | "organization">("organization")
  const [webhookSecret, setWebhookSecret] = useState("")
  const [error, setError] = useState<string | null>(null)
  const queryClient = useQueryClient()

  const { data: appConfig } = useQuery({
    queryKey: ["github-app-config"],
    queryFn: api.getGitHubAppConfig,
    staleTime: 5 * 60_000,
  })

  // GitHub redirects back with ?github_app_linked=N&tenant_id=... after a
  // successful install. Refresh this workspace's installations and strip
  // the query so a page reload doesn't loop.
  useEffect(() => {
    const params = new URLSearchParams(window.location.search)
    const linkedTenant = params.get("tenant_id")
    if (params.has("github_app_linked") && linkedTenant === workspaceId) {
      queryClient.invalidateQueries({
        queryKey: ["workspace-installations", workspaceId],
      })
      params.delete("github_app_linked")
      params.delete("tenant_id")
      const q = params.toString()
      window.history.replaceState(
        {},
        "",
        window.location.pathname + (q ? `?${q}` : ""),
      )
    }
  }, [workspaceId, queryClient])

  const add = useMutation({
    mutationFn: () =>
      api.registerWorkspaceInstallation(workspaceId, {
        install_id: Number(installId),
        account_login: accountLogin,
        account_type: accountType,
        webhook_secret: webhookSecret || undefined,
      }),
    onSuccess: () => {
      queryClient.invalidateQueries({
        queryKey: ["workspace-installations", workspaceId],
      })
      setAdding(false)
      setInstallId("")
      setAccountLogin("")
      setWebhookSecret("")
      setError(null)
    },
    onError: (err) => setError(err instanceof Error ? err.message : "Failed"),
  })

  const remove = useMutation({
    mutationFn: (id: string) => api.deleteWorkspaceInstallation(workspaceId, id),
    onSuccess: () => {
      queryClient.invalidateQueries({
        queryKey: ["workspace-installations", workspaceId],
      })
    },
  })

  return (
    <div className="space-y-2">
      <div className="flex items-center justify-between">
        <p className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
          <GitBranch className="mr-1 inline h-3 w-3" />
          GitHub installations ({installations.length})
        </p>
        {!adding && (
          <div className="flex items-center gap-2">
            {appConfig?.enabled && (
              <Button
                size="sm"
                onClick={() => {
                  window.location.href = `/api/github-app/install-start?tenant_id=${encodeURIComponent(workspaceId)}`
                }}
              >
                <Plus className="mr-1 h-3 w-3" />
                Install via GitHub App
              </Button>
            )}
            <Button
              size="sm"
              variant="outline"
              onClick={() => {
                setAdding(true)
                setError(null)
              }}
            >
              Link manually
            </Button>
          </div>
        )}
      </div>

      {installations.map((inst) => (
        <div
          key={inst.id}
          className="flex items-center justify-between gap-2 rounded-md bg-muted/50 p-2 text-xs"
        >
          <div className="min-w-0 flex-1">
            <p className="truncate font-mono">
              {inst.account_login} <span className="text-muted-foreground">({inst.account_type})</span>
            </p>
            <p className="text-muted-foreground">
              Installation #{inst.install_id}
            </p>
          </div>
          <Button
            size="sm"
            variant="outline"
            onClick={() => remove.mutate(inst.id)}
            disabled={remove.isPending}
            aria-label="Unlink installation"
          >
            <Trash2 className="h-3 w-3" />
          </Button>
        </div>
      ))}

      {adding && (
        <form
          onSubmit={(e) => {
            e.preventDefault()
            if (add.isPending) return
            if (installId && accountLogin) add.mutate()
          }}
          className="space-y-2 rounded-md border border-dashed p-2"
        >
          <div className="grid grid-cols-1 gap-2 sm:grid-cols-2">
            <Input
              placeholder="Installation ID (numeric)"
              value={installId}
              onChange={(e) =>
                setInstallId(e.target.value.replace(/\D/g, ""))
              }
            />
            <Input
              placeholder="Account login (org or user)"
              value={accountLogin}
              onChange={(e) => setAccountLogin(e.target.value)}
            />
          </div>
          <div className="grid grid-cols-1 gap-2 sm:grid-cols-2">
            <Select
              value={accountType}
              onValueChange={(v) =>
                setAccountType((v as "user" | "organization") ?? "organization")
              }
            >
              <SelectTrigger className="w-full">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="organization">Organization</SelectItem>
                <SelectItem value="user">User</SelectItem>
              </SelectContent>
            </Select>
            <Input
              type="password"
              placeholder="Webhook secret (optional)"
              value={webhookSecret}
              onChange={(e) => setWebhookSecret(e.target.value)}
            />
          </div>
          {error && <p className="text-xs text-destructive">{error}</p>}
          <div className="flex items-center gap-2">
            <Button
              size="sm"
              type="submit"
              disabled={!installId || !accountLogin || add.isPending}
            >
              Link
            </Button>
            <Button
              size="sm"
              type="button"
              variant="outline"
              onClick={() => {
                setAdding(false)
                setError(null)
              }}
            >
              Cancel
            </Button>
          </div>
        </form>
      )}
    </div>
  )
}

function ProjectsSection({
  workspaceId,
  installations,
  projects,
}: {
  workspaceId: string
  installations: GitHubAppInstallation[]
  projects: Project[]
}) {
  const [adding, setAdding] = useState(false)
  const [installationId, setInstallationId] = useState("")
  const [repoOwner, setRepoOwner] = useState("")
  const [repoName, setRepoName] = useState("")
  const [defaultBranch, setDefaultBranch] = useState("")
  const [manualEntry, setManualEntry] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const queryClient = useQueryClient()

  // Accessible-repos dropdown population. Only queried when the user has
  // started adding a project and selected an installation — the endpoint
  // returns 503 when the App isn't configured, which falls us through to
  // manual entry automatically.
  const { data: reposData, isLoading: reposLoading, isError: reposError } = useQuery({
    queryKey: ["installation-repos", workspaceId, installationId],
    queryFn: () =>
      installationId
        ? api.listInstallationRepos(workspaceId, installationId)
        : Promise.resolve({ repos: [] }),
    enabled: adding && !!installationId && !manualEntry,
    retry: false,
  })
  const repos = reposData?.repos ?? []

  const add = useMutation({
    mutationFn: () =>
      api.addWorkspaceProject(workspaceId, {
        github_app_installation_id: installationId,
        repo_owner: repoOwner,
        repo_name: repoName,
        default_branch: defaultBranch || undefined,
      }),
    onSuccess: () => {
      queryClient.invalidateQueries({
        queryKey: ["workspace-projects", workspaceId],
      })
      queryClient.invalidateQueries({ queryKey: ["all-projects"] })
      setAdding(false)
      setInstallationId("")
      setRepoOwner("")
      setRepoName("")
      setDefaultBranch("")
      setManualEntry(false)
      setError(null)
    },
    onError: (err) => setError(err instanceof Error ? err.message : "Failed"),
  })

  const remove = useMutation({
    mutationFn: (id: string) => api.deleteProject(id),
    onSuccess: () => {
      queryClient.invalidateQueries({
        queryKey: ["workspace-projects", workspaceId],
      })
      queryClient.invalidateQueries({ queryKey: ["all-projects"] })
    },
    onError: (err) => setError(err instanceof Error ? err.message : "Failed"),
  })

  const canAdd = installations.length > 0

  return (
    <div className="space-y-2">
      <div className="flex items-center justify-between">
        <p className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
          Projects ({projects.length})
        </p>
        {!adding && canAdd && (
          <Button
            size="sm"
            variant="outline"
            onClick={() => {
              setAdding(true)
              setError(null)
              setInstallationId(installations[0]?.id ?? "")
            }}
          >
            <Plus className="mr-1 h-3 w-3" />
            Add project
          </Button>
        )}
      </div>

      {!canAdd && (
        <p className="text-xs text-muted-foreground">
          Link a GitHub installation first to add projects.
        </p>
      )}

      {projects.map((p) => (
        <div
          key={p.id}
          className="flex items-center justify-between gap-2 rounded-md bg-muted/50 p-2 text-xs"
        >
          <div className="min-w-0 flex-1">
            <p className="truncate font-mono">
              {p.repo_owner}/{p.repo_name}
            </p>
            <p className="text-muted-foreground">
              default branch: {p.default_branch}
            </p>
          </div>
          <Button
            size="sm"
            variant="outline"
            onClick={() => remove.mutate(p.id)}
            disabled={remove.isPending}
            aria-label="Delete project"
          >
            <Trash2 className="h-3 w-3" />
          </Button>
        </div>
      ))}

      {adding && (
        <form
          onSubmit={(e) => {
            e.preventDefault()
            if (add.isPending) return
            if (installationId && repoOwner && repoName) add.mutate()
          }}
          className="space-y-2 rounded-md border border-dashed p-2"
        >
          <Select
            value={installationId}
            onValueChange={(v) => setInstallationId(v ?? "")}
          >
            <SelectTrigger className="w-full">
              <SelectValue placeholder="Select installation" />
            </SelectTrigger>
            <SelectContent>
              {installations.map((i) => (
                <SelectItem key={i.id} value={i.id}>
                  {i.account_login} ({i.account_type})
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          {!manualEntry && !reposError && repos.length > 0 ? (
            <Select
              value={repoOwner && repoName ? `${repoOwner}/${repoName}` : ""}
              onValueChange={(v) => {
                const picked = repos.find((r) => `${r.owner}/${r.name}` === v)
                if (picked) {
                  setRepoOwner(picked.owner)
                  setRepoName(picked.name)
                  setDefaultBranch(picked.default_branch ?? "")
                }
              }}
            >
              <SelectTrigger className="w-full">
                <SelectValue placeholder="Select a repository" />
              </SelectTrigger>
              <SelectContent>
                {repos.map((r) => (
                  <SelectItem key={`${r.owner}/${r.name}`} value={`${r.owner}/${r.name}`}>
                    {r.owner}/{r.name}
                    {r.private ? " (private)" : ""}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          ) : (
            <div className="grid grid-cols-1 gap-2 sm:grid-cols-2">
              <Input
                placeholder="Repo owner"
                value={repoOwner}
                onChange={(e) => setRepoOwner(e.target.value)}
              />
              <Input
                placeholder="Repo name"
                value={repoName}
                onChange={(e) => setRepoName(e.target.value)}
              />
            </div>
          )}
          {!manualEntry && reposLoading && (
            <p className="text-xs text-muted-foreground">Loading repositories…</p>
          )}
          {!manualEntry && reposError && (
            <p className="text-xs text-muted-foreground">
              Could not load accessible repositories — enter repo details manually.
            </p>
          )}
          {!manualEntry && !reposLoading && !reposError && repos.length > 0 && (
            <button
              type="button"
              className="text-xs text-muted-foreground underline"
              onClick={() => setManualEntry(true)}
            >
              enter repo manually instead
            </button>
          )}
          <Input
            placeholder="Default branch (optional — inferred from GitHub)"
            value={defaultBranch}
            onChange={(e) => setDefaultBranch(e.target.value)}
          />
          {error && <p className="text-xs text-destructive">{error}</p>}
          <div className="flex items-center gap-2">
            <Button
              size="sm"
              type="submit"
              disabled={
                !installationId || !repoOwner || !repoName || add.isPending
              }
            >
              Add
            </Button>
            <Button
              size="sm"
              type="button"
              variant="outline"
              onClick={() => {
                setAdding(false)
                setError(null)
              }}
            >
              Cancel
            </Button>
          </div>
        </form>
      )}
    </div>
  )
}
