import { useState } from "react"
import { useQuery } from "@tanstack/react-query"
import { Check, ChevronsUpDown, Plus, Tag } from "lucide-react"
import { api, type GitHubLabel } from "@/lib/api"
import { Button } from "@/components/ui/button"
import {
  Command,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
} from "@/components/ui/command"
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover"

export interface LabelComboboxProps {
  tenantId: string
  installId: string
  repoOwner: string
  repoName: string
  value: string | null
  onChange: (label: string) => void
  placeholder?: string
  /** Rendered-by-test prop hook: when provided, bypass the query and use
   *  these labels directly. Unused in production. */
  labelsOverride?: GitHubLabel[]
  /** Also a test hook — skip the react-query fetch. */
  disableFetch?: boolean
}

/**
 * A combobox that lists existing GitHub labels for the given repo and lets
 * the user pick one or create a new label inline. No free-text fallback —
 * the value committed is always a discrete label string.
 */
export function LabelCombobox({
  tenantId,
  installId,
  repoOwner,
  repoName,
  value,
  onChange,
  placeholder = "Select a label…",
  labelsOverride,
  disableFetch,
}: LabelComboboxProps) {
  const [open, setOpen] = useState(false)
  const [query, setQuery] = useState("")

  const fetchEnabled =
    !disableFetch &&
    !labelsOverride &&
    !!tenantId &&
    !!installId &&
    !!repoOwner &&
    !!repoName
  const { data, isLoading, isError } = useQuery({
    queryKey: ["repo-labels", tenantId, installId, repoOwner, repoName],
    queryFn: () => api.listRepoLabels(tenantId, installId, repoOwner, repoName),
    enabled: fetchEnabled,
    staleTime: 30_000,
    retry: false,
  })

  const labels: GitHubLabel[] = labelsOverride ?? data?.labels ?? []
  const trimmed = query.trim()
  const exactMatch = labels.some(
    (l) => l.name.toLowerCase() === trimmed.toLowerCase(),
  )
  const canCreate = trimmed.length > 0 && !exactMatch

  const commit = (name: string) => {
    onChange(name)
    setOpen(false)
    setQuery("")
  }

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger
        render={
          <Button
            variant="outline"
            role="combobox"
            aria-expanded={open}
            className="w-full justify-between"
          >
            <span className="flex min-w-0 items-center gap-2 truncate">
              <Tag className="h-4 w-4 shrink-0 text-muted-foreground" />
              {value ? (
                <span className="truncate">{value}</span>
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
      <PopoverContent className="w-[--radix-popover-trigger-width] p-0">
        <Command shouldFilter={false}>
          <CommandInput
            placeholder="Search or create a label…"
            value={query}
            onValueChange={setQuery}
          />
          <CommandList>
            {isLoading && !labelsOverride && (
              <div className="px-3 py-6 text-center text-sm text-muted-foreground">
                Loading labels…
              </div>
            )}
            {isError && !labelsOverride && (
              <div className="px-3 py-6 text-center text-sm text-destructive">
                Couldn&apos;t load labels. Check the GitHub install.
              </div>
            )}
            <CommandGroup heading="Existing labels">
              {labels
                .filter(
                  (l) =>
                    !trimmed ||
                    l.name.toLowerCase().includes(trimmed.toLowerCase()),
                )
                .map((l) => (
                  <CommandItem
                    key={l.name}
                    value={l.name}
                    onSelect={() => commit(l.name)}
                  >
                    <span
                      className="h-3 w-3 rounded-full border"
                      style={{
                        backgroundColor: l.color ? `#${l.color}` : undefined,
                      }}
                    />
                    <span className="truncate">{l.name}</span>
                    {value === l.name && (
                      <Check className="ml-auto h-4 w-4" />
                    )}
                  </CommandItem>
                ))}
            </CommandGroup>
            {!isLoading && labels.length === 0 && !canCreate && (
              <CommandEmpty>Type to create a label.</CommandEmpty>
            )}
            {canCreate && (
              <CommandGroup heading="Create">
                <CommandItem value={`__create:${trimmed}`} onSelect={() => commit(trimmed)}>
                  <Plus className="h-4 w-4" />
                  <span>Create label &quot;{trimmed}&quot;</span>
                </CommandItem>
              </CommandGroup>
            )}
          </CommandList>
        </Command>
      </PopoverContent>
    </Popover>
  )
}
