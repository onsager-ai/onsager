import { useEffect, useState } from "react"
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query"
import {
  api,
  type AccessibleRepo,
  type Workspace,
  type GitHubAppInstallation,
  type Project,
} from "@/lib/api"
import { Card, CardContent, CardHeader } from "@/components/ui/card"
import { Button, buttonVariants } from "@/components/ui/button"
import { Badge } from "@/components/ui/badge"
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
  Circle,
  GitBranch,
  Package,
  Plus,
  Trash2,
  Users,
} from "lucide-react"
import { cn } from "@/lib/utils"

/**
 * Card rendering a single workspace with its GitHub installations, projects,
 * and members. Setup state is surfaced up top so users can tell at a glance
 * where they are in the onboarding sequence.
 */
export function WorkspaceCard({ workspace }: { workspace: Workspace }) {
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

  const hasInstalls = installations.length > 0
  const hasProjects = projects.length > 0
  const fullySetUp = hasInstalls && hasProjects

  return (
    <Card>
      <CardHeader className="gap-3">
        <div className="flex flex-wrap items-start justify-between gap-3">
          <div className="min-w-0">
            <div className="flex items-center gap-2">
              <Building2 className="h-4 w-4 text-muted-foreground" />
              <h2 className="truncate text-lg font-semibold">{workspace.name}</h2>
            </div>
            <p className="mt-1 text-xs text-muted-foreground">
              <span className="font-mono">{workspace.slug}</span> · Created{" "}
              {new Date(workspace.created_at).toLocaleDateString()}
            </p>
          </div>
          {fullySetUp ? (
            <Badge variant="outline" className="shrink-0">
              <Check className="mr-1 h-3 w-3" />
              Ready
            </Badge>
          ) : (
            <Badge variant="secondary" className="shrink-0">
              Setup needed
            </Badge>
          )}
        </div>

        <SetupProgress
          hasInstalls={hasInstalls}
          hasProjects={hasProjects}
        />
      </CardHeader>
      <CardContent className="space-y-5">
        <MembersSection members={members} />

        <InstallationsSection
          workspaceId={workspace.id}
          installations={installations}
        />

        <ProjectsSection
          workspaceId={workspace.id}
          installations={installations}
          projects={projects}
        />
      </CardContent>
    </Card>
  )
}

function SetupProgress({
  hasInstalls,
  hasProjects,
}: {
  hasInstalls: boolean
  hasProjects: boolean
}) {
  const steps = [
    { label: "Workspace created", done: true },
    { label: "GitHub connected", done: hasInstalls },
    { label: "Project linked", done: hasProjects },
  ]
  return (
    <ol className="flex flex-wrap items-center gap-x-4 gap-y-2 text-xs">
      {steps.map((s, i) => (
        <li key={s.label} className="flex items-center gap-1.5">
          {s.done ? (
            <Check className="h-3.5 w-3.5 text-emerald-500" aria-hidden />
          ) : (
            <Circle className="h-3.5 w-3.5 text-muted-foreground" aria-hidden />
          )}
          <span
            className={cn(
              "font-medium",
              s.done ? "text-foreground" : "text-muted-foreground",
            )}
          >
            {i + 1}. {s.label}
          </span>
        </li>
      ))}
    </ol>
  )
}

function MembersSection({
  members,
}: {
  members: { user_id: string }[]
}) {
  return (
    <div className="space-y-1">
      <p className="flex items-center gap-1.5 text-xs font-medium uppercase tracking-wide text-muted-foreground">
        <Users className="h-3 w-3" />
        Members ({members.length})
      </p>
      <p className="text-xs text-muted-foreground">
        {members.map((m) => m.user_id).join(", ") || "—"}
      </p>
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

  const appMissing = appConfig && !appConfig.enabled

  return (
    <div className="space-y-2">
      <div className="flex items-center justify-between gap-2">
        <p className="flex items-center gap-1.5 text-xs font-medium uppercase tracking-wide text-muted-foreground">
          <GitBranch className="h-3 w-3" />
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

      {appMissing && installations.length === 0 && (
        <p className="rounded-md bg-muted/50 p-2 text-xs text-muted-foreground">
          GitHub App is not configured on this server. Ask an administrator to
          set up the Onsager GitHub App before linking a repository.
        </p>
      )}

      {!appMissing && installations.length === 0 && (
        <p className="rounded-md border border-dashed p-3 text-xs text-muted-foreground">
          No GitHub installations yet. Install the Onsager GitHub App on a
          user or organization to let this workspace manage its repositories.
        </p>
      )}

      {installations.map((inst) => (
        <div
          key={inst.id}
          className="flex items-center justify-between gap-2 rounded-md bg-muted/50 p-2 text-xs"
        >
          <div className="min-w-0 flex-1">
            <p className="truncate font-mono">
              {inst.account_login}{" "}
              <span className="text-muted-foreground">
                ({inst.account_type})
              </span>
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
      <div className="flex items-center justify-between gap-2">
        <p className="flex items-center gap-1.5 text-xs font-medium uppercase tracking-wide text-muted-foreground">
          <Package className="h-3 w-3" />
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
        <p className="rounded-md border border-dashed p-3 text-xs text-muted-foreground">
          Link a GitHub installation first — projects are picked from repos the
          Onsager GitHub App can see.
        </p>
      )}

      {canAdd && projects.length === 0 && !adding && (
        <p className="rounded-md border border-dashed p-3 text-xs text-muted-foreground">
          No projects yet. Add a repo to start running agent sessions against
          it.
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
          className="space-y-2 rounded-md border border-dashed p-3"
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
      <PopoverContent className="w-[var(--anchor-width)] min-w-64 p-0" align="start">
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
                    data-checked={selected === value}
                    onSelect={() => {
                      onSelect(r)
                      setOpen(false)
                    }}
                  >
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
