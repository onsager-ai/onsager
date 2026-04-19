import { useEffect, useState } from "react"
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query"
import {
  api,
  type AccessibleRepo,
  type Workspace,
  type GitHubAppInstallation,
  type Project,
} from "@/lib/api"
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/card"
import { Button, buttonVariants } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover"
import {
  Command,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
} from "@/components/ui/command"
import {
  Building2,
  Check,
  ChevronsUpDown,
  GitBranch,
  Plus,
  Trash2,
} from "lucide-react"
import { cn } from "@/lib/utils"

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
      </div>

      {appConfig && !appConfig.enabled && installations.length === 0 && (
        <p className="rounded-md bg-muted/50 p-2 text-xs text-muted-foreground">
          GitHub App is not configured on this server. Ask an administrator to
          set up the Onsager GitHub App before linking a repository.
        </p>
      )}

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
    enabled: adding && !!installationId,
    retry: false,
  })
  const repos = reposData?.repos ?? []
  const selectedInstallation = installations.find((i) => i.id === installationId)
  const configureUrl = selectedInstallation
    ? selectedInstallation.account_type === "organization"
      ? `https://github.com/organizations/${selectedInstallation.account_login}/settings/installations/${selectedInstallation.install_id}`
      : `https://github.com/settings/installations/${selectedInstallation.install_id}`
    : null

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
          data-testid="add-project-form"
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
          {installationId && reposLoading && (
            <p className="text-xs text-muted-foreground">Loading repositories…</p>
          )}
          {installationId && !reposLoading && repos.length > 0 && (
            <RepoCombobox
              repos={repos}
              selected={
                repoOwner && repoName ? `${repoOwner}/${repoName}` : ""
              }
              onSelect={(picked) => {
                setRepoOwner(picked.owner)
                setRepoName(picked.name)
                setDefaultBranch(picked.default_branch ?? "")
              }}
            />
          )}
          {installationId && !reposLoading && repos.length === 0 && configureUrl && (
            <div className="space-y-1 rounded-md bg-muted/50 p-2 text-xs">
              <p className="text-muted-foreground">
                {reposError
                  ? "Could not load repositories for this installation."
                  : "This installation has no repositories yet."}
              </p>
              <a
                href={configureUrl}
                target="_blank"
                rel="noopener noreferrer"
                className="inline-flex items-center gap-1 font-medium text-primary underline"
              >
                Configure repository access on GitHub →
              </a>
            </div>
          )}
          {installationId && !reposLoading && repos.length > 0 && configureUrl && (
            <a
              href={configureUrl}
              target="_blank"
              rel="noopener noreferrer"
              className="inline-flex items-center gap-1 text-xs text-muted-foreground underline"
            >
              Don't see your repo? Configure on GitHub →
            </a>
          )}
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

function RepoCombobox({
  repos,
  selected,
  onSelect,
}: {
  repos: AccessibleRepo[]
  selected: string
  onSelect: (repo: AccessibleRepo) => void
}) {
  const [open, setOpen] = useState(false)
  const selectedRepo = repos.find((r) => `${r.owner}/${r.name}` === selected)

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger
        role="combobox"
        aria-expanded={open}
        aria-label="Select a repository"
        className={cn(
          buttonVariants({ variant: "outline" }),
          "w-full justify-between font-normal",
        )}
      >
        <span className={cn("truncate", !selectedRepo && "text-muted-foreground")}>
          {selectedRepo
            ? `${selectedRepo.owner}/${selectedRepo.name}`
            : "Select a repository"}
        </span>
        <ChevronsUpDown className="ml-2 h-4 w-4 shrink-0 opacity-50" />
      </PopoverTrigger>
      <PopoverContent className="w-[--radix-popover-trigger-width] p-0" align="start">
        <Command>
          <CommandInput placeholder="Search repositories…" />
          <CommandList>
            <CommandEmpty>No repositories match.</CommandEmpty>
            <CommandGroup>
              {repos.map((r) => {
                const value = `${r.owner}/${r.name}`
                return (
                  <CommandItem
                    key={value}
                    value={value}
                    onSelect={() => {
                      onSelect(r)
                      setOpen(false)
                    }}
                  >
                    <Check
                      className={cn(
                        "mr-2 h-4 w-4",
                        selected === value ? "opacity-100" : "opacity-0",
                      )}
                    />
                    <span className="truncate font-mono text-xs">
                      {r.owner}/{r.name}
                    </span>
                    {r.private && (
                      <span className="ml-2 text-[10px] uppercase text-muted-foreground">
                        private
                      </span>
                    )}
                  </CommandItem>
                )
              })}
            </CommandGroup>
          </CommandList>
        </Command>
      </PopoverContent>
    </Popover>
  )
}
