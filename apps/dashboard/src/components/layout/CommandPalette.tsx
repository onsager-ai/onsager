import {
  createContext,
  useContext,
  useEffect,
  useMemo,
  useState,
  type ReactNode,
} from "react"
import { useNavigate } from "react-router-dom"
import {
  Activity,
  Building2,
  Factory,
  GitBranch,
  Package,
  Search,
  Server,
  Settings as SettingsIcon,
  Shield,
  Terminal,
  Zap,
} from "lucide-react"
import { Button } from "@/components/ui/button"
import {
  Command,
  CommandDialog,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
  CommandSeparator,
} from "@/components/ui/command"
import { CreateSessionSheet } from "@/components/sessions/CreateSessionSheet"
import { NewWorkspaceDialog } from "@/components/workspaces/NewWorkspaceDialog"
import { useAuth } from "@/lib/auth"

// Single global "do something" surface — create a primitive or jump to
// a section. Replaces the old chrome `+` dropdown: that pattern doesn't
// scale (each new primitive forces a chrome redesign), and on info-only
// pages had nothing to offer. Convention-aligned with Linear / Slack /
// Vercel / Notion command palettes.
//
// Architecture: one <CommandPaletteProvider> mounts the dialog once;
// chrome surfaces use <CommandPaletteTrigger /> to render the search
// icon. Sharing state via context avoids two dialog instances (mobile
// + desktop chrome) doubling up the ⌘K listener.

interface PaletteController {
  open: boolean
  setOpen: (open: boolean) => void
}

const PaletteContext = createContext<PaletteController | null>(null)

function usePalette(): PaletteController {
  const ctx = useContext(PaletteContext)
  if (!ctx) throw new Error("usePalette must be used within CommandPaletteProvider")
  return ctx
}

export function CommandPaletteProvider({ children }: { children: ReactNode }) {
  const [open, setOpen] = useState(false)

  // Global ⌘K / Ctrl+K hotkey. Bound to window so it works regardless
  // of which page is focused.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "k" && (e.metaKey || e.ctrlKey)) {
        e.preventDefault()
        setOpen((v) => !v)
      }
    }
    window.addEventListener("keydown", onKey)
    return () => window.removeEventListener("keydown", onKey)
  }, [])

  const value = useMemo<PaletteController>(() => ({ open, setOpen }), [open])

  return (
    <PaletteContext.Provider value={value}>
      {children}
      <CommandPaletteDialog />
    </PaletteContext.Provider>
  )
}

export function CommandPaletteTrigger() {
  const { setOpen } = usePalette()
  return (
    <Button
      variant="ghost"
      size="icon"
      className="h-9 w-9"
      aria-label="Open command palette (⌘K)"
      title="Search and create (⌘K)"
      onClick={() => setOpen(true)}
    >
      <Search className="h-5 w-5" />
    </Button>
  )
}

function CommandPaletteDialog() {
  const { open, setOpen } = usePalette()
  const navigate = useNavigate()
  const { user, authEnabled } = useAuth()
  const canAuth = authEnabled && !!user

  const [sessionOpen, setSessionOpen] = useState(false)
  const [workspaceOpen, setWorkspaceOpen] = useState(false)

  const run = (action: () => void) => {
    setOpen(false)
    action()
  }
  const go = (path: string) => run(() => navigate(path))

  return (
    <>
      <CommandDialog open={open} onOpenChange={setOpen}>
        {/* shadcn's CommandDialog wraps in <DialogContent> but NOT in
            <Command>; the cmdk primitives below need a Command parent
            for context, otherwise CommandInput throws and the dialog
            tears down the tree (blank screen). */}
        <Command>
          <CommandInput placeholder="Type a command or search…" />
          <CommandList>
          <CommandEmpty>No results.</CommandEmpty>

            <CommandGroup heading="Create">
              <CommandItem
                keywords={["new", "factory", "github"]}
                onSelect={() => go("/workflows/start")}
              >
                <Zap className="mr-2 h-4 w-4" />
                New workflow
              </CommandItem>
              {canAuth && (
                <CommandItem
                  keywords={["new"]}
                  onSelect={() => run(() => setSessionOpen(true))}
                >
                  <Terminal className="mr-2 h-4 w-4" />
                  New session
                </CommandItem>
              )}
              {canAuth && (
                <CommandItem
                  keywords={["new", "tenant"]}
                  onSelect={() => run(() => setWorkspaceOpen(true))}
                >
                  <Building2 className="mr-2 h-4 w-4" />
                  New workspace
                </CommandItem>
              )}
            </CommandGroup>

            <CommandSeparator />

            <CommandGroup heading="Go to">
              <CommandItem keywords={["overview"]} onSelect={() => go("/")}>
                <Factory className="mr-2 h-4 w-4" />
                Factory overview
              </CommandItem>
              {canAuth && (
                <CommandItem onSelect={() => go("/workspaces")}>
                  <Building2 className="mr-2 h-4 w-4" />
                  Workspaces
                </CommandItem>
              )}
              <CommandItem onSelect={() => go("/workflows")}>
                <GitBranch className="mr-2 h-4 w-4" />
                Workflows
              </CommandItem>
              <CommandItem onSelect={() => go("/artifacts")}>
                <Package className="mr-2 h-4 w-4" />
                Artifacts
              </CommandItem>
              <CommandItem onSelect={() => go("/spine")}>
                <Activity className="mr-2 h-4 w-4" />
                Event spine
              </CommandItem>
              <CommandItem onSelect={() => go("/governance")}>
                <Shield className="mr-2 h-4 w-4" />
                Governance
              </CommandItem>
              <CommandItem onSelect={() => go("/sessions")}>
                <Terminal className="mr-2 h-4 w-4" />
                Sessions
              </CommandItem>
              <CommandItem onSelect={() => go("/nodes")}>
                <Server className="mr-2 h-4 w-4" />
                Nodes
              </CommandItem>
              <CommandItem onSelect={() => go("/settings")}>
                <SettingsIcon className="mr-2 h-4 w-4" />
                Settings
              </CommandItem>
            </CommandGroup>
          </CommandList>
        </Command>
      </CommandDialog>

      {sessionOpen && (
        <CreateSessionSheet open={sessionOpen} onOpenChange={setSessionOpen} />
      )}
      {canAuth && (
        <NewWorkspaceDialog
          open={workspaceOpen}
          onOpenChange={setWorkspaceOpen}
        />
      )}
    </>
  )
}
