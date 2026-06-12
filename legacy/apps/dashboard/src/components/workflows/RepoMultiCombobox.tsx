import { useMemo, useState } from "react"
import { useQueries } from "@tanstack/react-query"
import { Check, ChevronsUpDown, ExternalLink, Folder, Lock, X } from "lucide-react"
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
import type { RepoRef } from "./workflow-draft"

export interface RepoMultiComboboxProps {
  workspaceId: string
  installations: GitHubAppInstallation[]
  value: RepoRef[]
  onChange: (next: RepoRef[]) => void
}

function sameRepo(a: RepoRef, b: RepoRef): boolean {
  return (
    a.install_id === b.install_id &&
    a.repo_owner === b.repo_owner &&
    a.repo_name === b.repo_name
  )
}

/**
 * Multi-select sibling of `RepoCombobox`: pick one *or more* GitHub repos as
 * trigger sources (#566). Same parallel fan-out across workspace installs and
 * the same account-grouped list, but selecting a repo toggles it in/out of
 * the set and keeps the popover open so several can be chosen in a row.
 * Selected repos show as removable chips above the trigger button; the order
 * is preserved so `value[0]` stays the canonical primary repo the create
 * request emits today.
 */
export function RepoMultiCombobox({
  workspaceId,
  installations,
  value,
  onChange,
}: RepoMultiComboboxProps) {
  const [open, setOpen] = useState(false)

  // Gate the fan-out on the popover being open — see RepoCombobox for the
  // rationale (avoid N requests per render; staleTime keeps reopens snappy).
  const queries = useQueries({
    queries: installations.map((inst) => ({
      queryKey: ["installation-repos", workspaceId, inst.id],
      queryFn: () => api.listInstallationRepos(workspaceId, inst.id),
      enabled: !!workspaceId && open,
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
  const hasError = queries.some((q) => q.isError)
  const allErrored = queries.length > 0 && queries.every((q) => q.isError)

  const configureInstall = installations[0] ?? null
  const configureUrl = configureInstall
    ? configureInstall.account_type === "organization"
      ? `https://github.com/organizations/${configureInstall.account_login}/settings/installations/${configureInstall.install_id}`
      : `https://github.com/settings/installations/${configureInstall.install_id}`
    : null

  const toggle = (
    install: GitHubAppInstallation,
    repo: { owner: string; name: string },
  ) => {
    const ref: RepoRef = {
      install_id: install.id,
      repo_owner: repo.owner,
      repo_name: repo.name,
    }
    const exists = value.some((r) => sameRepo(r, ref))
    onChange(exists ? value.filter((r) => !sameRepo(r, ref)) : [...value, ref])
  }

  const remove = (ref: RepoRef) => {
    onChange(value.filter((r) => !sameRepo(r, ref)))
  }

  const disabled = installations.length === 0
  const triggerLabel = disabled
    ? "No GitHub App installs yet"
    : value.length === 0
      ? "Pick repositories"
      : `${value.length} ${value.length === 1 ? "repository" : "repositories"} selected`

  return (
    <div className="space-y-2">
      {value.length > 0 && (
        <ul className="flex flex-wrap gap-1.5">
          {value.map((ref, i) => (
            <li
              key={`${ref.install_id}:${ref.repo_owner}/${ref.repo_name}`}
              className="flex items-center gap-1.5 rounded-md border bg-muted/40 py-1 pl-2 pr-1 text-xs"
            >
              <Folder className="h-3 w-3 shrink-0 text-muted-foreground" />
              <span className="truncate">
                {ref.repo_owner}/{ref.repo_name}
              </span>
              {i === 0 && (
                <span className="rounded bg-primary/10 px-1 text-[10px] uppercase tracking-wide text-primary">
                  primary
                </span>
              )}
              <button
                type="button"
                aria-label={`Remove ${ref.repo_owner}/${ref.repo_name}`}
                className="rounded-sm p-0.5 text-muted-foreground hover:bg-muted hover:text-foreground"
                onClick={() => remove(ref)}
              >
                <X className="h-3 w-3" />
              </button>
            </li>
          ))}
        </ul>
      )}
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
                <span
                  className={
                    value.length === 0 ? "truncate text-muted-foreground" : "truncate"
                  }
                >
                  {triggerLabel}
                </span>
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
              {!isLoading && allErrored && (
                <div className="px-3 py-6 text-center text-sm text-destructive">
                  Couldn&apos;t load repositories. Check the GitHub install.
                </div>
              )}
              {!isLoading && hasError && !allErrored && (
                <div className="px-3 py-2 text-xs text-destructive">
                  Couldn&apos;t load some installs — list may be incomplete.
                </div>
              )}
              {!isLoading && !allErrored && (
                <CommandEmpty>
                  {totalRepos === 0
                    ? "No repositories accessible to this install."
                    : "No matching repositories."}
                </CommandEmpty>
              )}
              {grouped.map(({ install, repos }) =>
                repos.length === 0 ? null : (
                  <CommandGroup
                    key={install.id}
                    heading={`${install.account_login} (${install.account_type})`}
                  >
                    {repos.map((repo) => {
                      const isSelected = value.some((r) =>
                        sameRepo(r, {
                          install_id: install.id,
                          repo_owner: repo.owner,
                          repo_name: repo.name,
                        }),
                      )
                      return (
                        <CommandItem
                          key={`${install.id}:${repo.owner}/${repo.name}`}
                          value={`${install.account_login} ${repo.owner}/${repo.name}`}
                          onSelect={() => toggle(install, repo)}
                        >
                          <Folder className="h-4 w-4 shrink-0 text-muted-foreground" />
                          <span className="truncate">
                            {repo.owner}/{repo.name}
                          </span>
                          {repo.private && (
                            <Lock className="h-3 w-3 shrink-0 text-muted-foreground" />
                          )}
                          {isSelected && <Check className="ml-auto h-4 w-4" />}
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
    </div>
  )
}
