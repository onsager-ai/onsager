import { useMemo, useState } from "react"
import { useQuery } from "@tanstack/react-query"
import { Check, ChevronsUpDown, Layers } from "lucide-react"
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
import { mcpCallTool } from "@/lib/mcp-client"

// The `kind` picker for a spec node (spec #542). A spec's `kind` must be
// an *active* entry in the workspace's workflow library — the same set
// `compile_dry_run` resolves against. Rather than free-typing a kind
// (and learning it's wrong only at compile time), this reads the library
// over the existing `list_workflows_v2` MCP tool and offers it as a
// searchable combobox, per the dashboard-ui "avoid manual input" rule.
// No new REST endpoint, no parallel kind list: the library is the one
// source of truth, reached through the same MCP client the chat uses.

interface WorkflowLibraryCard {
  spec_kind: string
  version: number
  retired_at: string | null
}

/** Read active library kinds (retired entries filtered out), de-duplicated
 *  and sorted. Returns `[]` while loading or on error so the picker stays
 *  mountable; the caller surfaces the load failure. */
function useSpecKinds(workspaceId: string) {
  const query = useQuery({
    queryKey: ["workflow-library-kinds", workspaceId],
    queryFn: async () => {
      const res = await mcpCallTool("list_workflows_v2", {
        workspace_id: workspaceId,
      })
      if (res.isError) {
        throw new Error(res.content[0]?.text ?? "Failed to list workflow kinds")
      }
      const parsed = JSON.parse(res.content[0]?.text ?? "{}") as {
        workflows?: WorkflowLibraryCard[]
      }
      const active = (parsed.workflows ?? []).filter((c) => c.retired_at == null)
      return [...new Set(active.map((c) => c.spec_kind))].sort()
    },
    enabled: !!workspaceId,
    staleTime: 60_000,
  })
  return {
    kinds: query.data ?? [],
    isLoading: query.isLoading,
    isError: query.isError,
  }
}

export interface SpecKindComboboxProps {
  workspaceId: string
  value: string
  onChange: (kind: string) => void
  /** Accessible name for the trigger (each row's picker is distinct). */
  ariaLabel?: string
}

export function SpecKindCombobox({
  workspaceId,
  value,
  onChange,
  ariaLabel,
}: SpecKindComboboxProps) {
  const [open, setOpen] = useState(false)
  const { kinds, isLoading, isError } = useSpecKinds(workspaceId)

  const empty = useMemo(() => {
    if (isLoading) return "Loading kinds…"
    if (isError) return "Couldn't load the workflow library."
    return "No matching kinds. Register one in the library first."
  }, [isLoading, isError])

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger
        render={
          <Button
            variant="outline"
            role="combobox"
            aria-expanded={open}
            aria-label={ariaLabel ?? "Spec kind"}
            className="w-full justify-between"
          >
            <span className="flex min-w-0 items-center gap-2 truncate">
              <Layers className="h-4 w-4 shrink-0 text-muted-foreground" />
              {value ? (
                <span className="truncate">{value}</span>
              ) : (
                <span className="truncate text-muted-foreground">Pick a kind</span>
              )}
            </span>
            <ChevronsUpDown className="h-4 w-4 shrink-0 opacity-50" />
          </Button>
        }
      />
      <PopoverContent
        className="w-[--radix-popover-trigger-width] min-w-56 p-0"
        align="start"
      >
        <Command>
          <CommandInput placeholder="Search kinds…" />
          <CommandList>
            <CommandEmpty>{empty}</CommandEmpty>
            <CommandGroup>
              {kinds.map((kind) => (
                <CommandItem
                  key={kind}
                  value={kind}
                  onSelect={() => {
                    onChange(kind)
                    setOpen(false)
                  }}
                >
                  <Layers className="h-4 w-4 shrink-0 text-muted-foreground" />
                  <span className="truncate">{kind}</span>
                  {value === kind && <Check className="ml-auto h-4 w-4" />}
                </CommandItem>
              ))}
            </CommandGroup>
          </CommandList>
        </Command>
      </PopoverContent>
    </Popover>
  )
}
