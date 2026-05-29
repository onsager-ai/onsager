import { useMemo, useState } from "react"
import { useLocation, useNavigate } from "react-router-dom"
import { Building2, Check, ChevronDown, Plus } from "lucide-react"

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
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover"
import { NewWorkspaceDialog } from "@/components/workspaces/NewWorkspaceDialog"
import {
  useMembershipWorkspaces,
  useOptionalActiveWorkspace,
} from "@/lib/workspace"
import type { Workspace } from "@/lib/api"
import { cn } from "@/lib/utils"

// Resource segments we know how to preserve when switching workspaces.
// `/workspaces/W1/sessions/abc` should switch to `/workspaces/W2/sessions`
// (drop the W1-specific row id) rather than `/workspaces/W2`.
const RESOURCE_SEGMENTS = new Set([
  "sessions",
  "nodes",
  "artifacts",
  "workflows",
  "governance",
  "spine",
  "issues",
  "settings",
])

function targetPathForSwitch(currentPath: string, nextSlug: string): string {
  // Match `/workspaces/<slug>/<rest...>`. If we're not on a scoped path
  // (e.g. user opened the switcher from /workspaces), land on the
  // workspace's overview.
  const m = currentPath.match(/^\/workspaces\/[^/]+(?:\/(.*))?$/)
  if (!m) return `/workspaces/${nextSlug}`
  const rest = m[1] ?? ""
  if (!rest) return `/workspaces/${nextSlug}`
  const top = rest.split("/")[0]
  if (RESOURCE_SEGMENTS.has(top)) {
    return `/workspaces/${nextSlug}/${top}`
  }
  return `/workspaces/${nextSlug}`
}

function WorkspaceBadge({ workspace }: { workspace: Workspace | null }) {
  if (!workspace) {
    return (
      <span className="flex h-6 w-6 shrink-0 items-center justify-center rounded-md border bg-muted text-muted-foreground">
        <Building2 className="h-3.5 w-3.5" />
      </span>
    )
  }
  // First letter of the slug (lowercased) gives a stable "avatar" without
  // requiring a generated image. Avoids manual-input territory while still
  // distinguishing workspaces visually in the picker rows.
  const initial = (workspace.slug[0] ?? "?").toUpperCase()
  return (
    <span className="flex h-6 w-6 shrink-0 items-center justify-center rounded-md bg-primary/10 text-xs font-semibold text-primary">
      {initial}
    </span>
  )
}

export function WorkspaceSwitcher() {
  const navigate = useNavigate()
  const location = useLocation()
  const workspaces = useMembershipWorkspaces()
  const active = useOptionalActiveWorkspace()
  const [open, setOpen] = useState(false)
  const [createOpen, setCreateOpen] = useState(false)

  // Sort by slug so the picker order is stable across renders.
  const sorted = useMemo(
    () => [...workspaces].sort((a, b) => a.slug.localeCompare(b.slug)),
    [workspaces],
  )

  if (workspaces.length === 0) {
    // Zero memberships: the FTUE flow lives at the workspace picker
    // (`/workspaces`) and binding lifts workspace creation into a
    // single dialog. Rendering a no-op switcher here would be
    // confusing chrome.
    return null
  }

  const switchTo = (workspace: Workspace) => {
    setOpen(false)
    navigate(targetPathForSwitch(location.pathname, workspace.slug))
  }

  return (
    <>
      <Popover open={open} onOpenChange={setOpen}>
        <PopoverTrigger
          render={
            <Button
              variant="ghost"
              size="sm"
              role="combobox"
              aria-expanded={open}
              aria-label="Switch workspace"
              className="h-8 gap-1.5 px-2 text-sm font-medium"
            >
              <span className="max-w-[12rem] truncate">
                {active ? active.name : "Select workspace"}
              </span>
              <ChevronDown className="h-3.5 w-3.5 shrink-0 opacity-50" />
            </Button>
          }
        />
        <PopoverContent
          className="w-[--radix-popover-trigger-width] min-w-64 p-0"
          align="start"
        >
          <Command>
            <CommandInput placeholder="Find a workspace…" />
            <CommandList>
              <CommandEmpty>No workspaces found.</CommandEmpty>
              <CommandGroup heading="Workspaces">
                {sorted.map((w) => (
                  <CommandItem
                    key={w.id}
                    value={`${w.slug} ${w.name}`}
                    onSelect={() => switchTo(w)}
                  >
                    <WorkspaceBadge workspace={w} />
                    <span className="ml-2 min-w-0 flex-1">
                      <span className="block truncate text-sm">{w.name}</span>
                      <span className="block truncate text-xs text-muted-foreground">
                        {w.slug}
                      </span>
                    </span>
                    <Check
                      className={cn(
                        "ml-2 h-4 w-4",
                        active?.id === w.id ? "opacity-100" : "opacity-0",
                      )}
                    />
                  </CommandItem>
                ))}
              </CommandGroup>
              <CommandSeparator />
              <CommandGroup>
                <CommandItem
                  onSelect={() => {
                    setOpen(false)
                    setCreateOpen(true)
                  }}
                >
                  <Plus className="mr-2 h-4 w-4" />
                  Create workspace
                </CommandItem>
              </CommandGroup>
            </CommandList>
          </Command>
        </PopoverContent>
      </Popover>
      <NewWorkspaceDialog open={createOpen} onOpenChange={setCreateOpen} />
    </>
  )
}
