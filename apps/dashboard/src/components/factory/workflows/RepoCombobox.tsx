import { useMemo, useState } from "react"
import { useQueries } from "@tanstack/react-query"
import { Check, ChevronsUpDown, ExternalLink, Folder, Lock } from "lucide-react"
import { api, type GitHubAppInstallation } from "@/lib/api"
import { Button } from "@/components/ui/button"
import {
  Command,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
  CommandSeparator,
} from "@/components/ui/command"
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover"

export interface RepoComboboxProps {
  tenantId: string
  installations: GitHubAppInstallation[]
  installId: string
  repoOwner: string
  repoName: string
  onChange: (next: {
    install_id: string
    repo_owner: string
    repo_name: string
  }) => void
}

/**
 * One picker for "what GitHub repo triggers this workflow." Replaces the
 * cascading install + repo selects: we fan out across every workspace
 * install in parallel, group results under each install's account login,
 * and emit all three identifiers (install record id, owner, name) on
 * select. Linkable fields stay un-typed, per the dashboard-ui rules.
 */
export function RepoCombobox({
  tenantId,
  installations,
  installId,
  repoOwner,
  repoName,
  onChange,
}: RepoComboboxProps) {
  const [open, setOpen] = useState(false)

  const queries = useQueries({
    queries: installations.map((inst) => ({
      queryKey: ["installation-repos", tenantId, inst.id],
      queryFn: () => api.listInstallationRepos(tenantId, inst.id),
      enabled: !!tenantId,
      staleTime: 30_000,
      retry: false,
    })),
  })

  const grouped = useMemo(
    () =>
      installations.map((install, i) => ({
        install,
        repos: queries[i]?.data?.repos ?? [],
      })),
    [installations, queries],
  )

  const totalRepos = grouped.reduce((sum, g) => sum + g.repos.length, 0)
  const isLoading = queries.some((q) => q.isLoading)
  const isError = queries.length > 0 && queries.every((q) => q.isError)

  const selectedLabel =
    repoOwner && repoName ? `${repoOwner}/${repoName}` : null

  const selectedInstall = installations.find((i) => i.id === installId)
  const configureInstall = selectedInstall ?? installations[0] ?? null
  const configureUrl = configureInstall
    ? configureInstall.account_type === "organization"
      ? `https://github.com/organizations/${configureInstall.account_login}/settings/installations/${configureInstall.install_id}`
      : `https://github.com/settings/installations/${configureInstall.install_id}`
    : null

  const commit = (
    install: GitHubAppInstallation,
    repo: { owner: string; name: string },
  ) => {
    onChange({
      install_id: install.id,
      repo_owner: repo.owner,
      repo_name: repo.name,
    })
    setOpen(false)
  }

  const disabled = installations.length === 0
  const placeholder = disabled
    ? "No GitHub App installs yet"
    : "Pick a repository"

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger
        render={
          <Button
            variant="outline"
            role="combobox"
            aria-expanded={open}
            className="w-full justify-between"
            disabled={disabled}
          >
            <span className="flex min-w-0 items-center gap-2 truncate">
              <Folder className="h-4 w-4 shrink-0 text-muted-foreground" />
              {selectedLabel ? (
                <span className="truncate">{selectedLabel}</span>
              ) : (
                <span className="truncate text-muted-foreground">
                  {placeholder}
                </span>
              )}
            </span>
            <ChevronsUpDown className="h-4 w-4 shrink-0 opacity-50" />
          </Button>
        }
      />
      <PopoverContent
        className="w-[--radix-popover-trigger-width] min-w-72 p-0"
        align="start"
      >
        <Command>
          <CommandInput placeholder="Search repositories…" />
          <CommandList>
            {isLoading && totalRepos === 0 && (
              <div className="px-3 py-6 text-center text-sm text-muted-foreground">
                Loading repositories…
              </div>
            )}
            {!isLoading && isError && (
              <div className="px-3 py-6 text-center text-sm text-destructive">
                Couldn&apos;t load repositories. Check the GitHub install.
              </div>
            )}
            {!isLoading && !isError && totalRepos === 0 && (
              <CommandEmpty>
                No repositories accessible to this install.
              </CommandEmpty>
            )}
            {grouped.map(({ install, repos }) =>
              repos.length === 0 ? null : (
                <CommandGroup
                  key={install.id}
                  heading={`${install.account_login} (${install.account_type})`}
                >
                  {repos.map((repo) => {
                    const isSelected =
                      install.id === installId &&
                      repo.owner === repoOwner &&
                      repo.name === repoName
                    return (
                      <CommandItem
                        key={`${install.id}:${repo.owner}/${repo.name}`}
                        value={`${install.account_login} ${repo.owner}/${repo.name}`}
                        onSelect={() => commit(install, repo)}
                      >
                        <Folder className="h-4 w-4 shrink-0 text-muted-foreground" />
                        <span className="truncate">
                          {repo.owner}/{repo.name}
                        </span>
                        {repo.private && (
                          <Lock className="h-3 w-3 shrink-0 text-muted-foreground" />
                        )}
                        {isSelected && (
                          <Check className="ml-auto h-4 w-4" />
                        )}
                      </CommandItem>
                    )
                  })}
                </CommandGroup>
              ),
            )}
            {configureUrl && (
              <>
                <CommandSeparator />
                <CommandGroup>
                  <CommandItem
                    value="__configure-github-access"
                    onSelect={() => {
                      window.open(
                        configureUrl,
                        "_blank",
                        "noopener,noreferrer",
                      )
                      setOpen(false)
                    }}
                  >
                    <ExternalLink className="h-4 w-4 shrink-0 text-muted-foreground" />
                    <span className="truncate">
                      Configure repository access on GitHub
                    </span>
                  </CommandItem>
                </CommandGroup>
              </>
            )}
          </CommandList>
        </Command>
      </PopoverContent>
    </Popover>
  )
}
