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
  GitBranch,
  MessageSquare,
  Search,
  Settings as SettingsIcon,
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
import { useOptionalActiveWorkspace } from "@/lib/workspace"
import { useOptionalChatUi } from "@/lib/chat/use-chat-ui"

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

// Parses the palette input for the chat-invocation grammar (ADR 0020
// channel 5, spec #473). Returns the inline message after `ask:` /
// `ask ` (empty string when the user typed only the keyword), or null
// when the input is not an ask command. Matches case-insensitively so
// `Ask:` and `ASK ` work the same. The `\b` anchor keeps `askbar`
// from triggering — only the word `ask` followed by end-of-input,
// `:`, or whitespace counts.
// eslint-disable-next-line react-refresh/only-export-components
export function parseAskCommand(input: string): string | null {
  const trimmed = input.trim()
  if (!trimmed) return null
  const m = trimmed.match(/^ask\b\s*:?\s*(.*)$/i)
  if (!m) return null
  return m[1].trim()
}

function CommandPaletteDialog() {
  const { open, setOpen } = usePalette()
  const navigate = useNavigate()
  const activeWorkspace = useOptionalActiveWorkspace()
  const chatUi = useOptionalChatUi()

  const [sessionOpen, setSessionOpen] = useState(false)
  const [workspaceOpen, setWorkspaceOpen] = useState(false)
  const [search, setSearch] = useState("")

  // Detected "ask…" grammar in the input (ADR 0020 channel 5):
  //   "ask"          → empty prefill (just opens the chat)
  //   "ask:"         → empty prefill
  //   "ask: <msg>"   → seed the composer with <msg>
  //   "ask <msg>"    → seed the composer with <msg>
  // When non-null we force-mount the chat command so cmdk's fuzzy
  // filter doesn't drop it for inputs the score wouldn't otherwise
  // match (e.g. "ask: why did run #42 fail?").
  const askPrefill = useMemo(() => parseAskCommand(search), [search])
  const hasAskGrammar = askPrefill !== null
  const showInlineLabel = hasAskGrammar && (askPrefill ?? "").length > 0

  const run = (action: () => void) => {
    setOpen(false)
    setSearch("")
    action()
  }
  const go = (path: string) => run(() => navigate(path))
  // Scope deep-links to the active workspace when there is one;
  // otherwise route to the workspace picker so the user lands on a
  // surface that can resolve the missing context (auth is always-on
  // post-#193, so the picker is always reachable).
  const scoped = (suffix: string) =>
    activeWorkspace
      ? `/workspaces/${activeWorkspace.slug}/${suffix}`
      : "/workspaces"

  // Open the global chat (ADR 0020 / spec #473). Routes the prefill
  // through `useChatUi.open({ prefill })` — the chat lands on Thread
  // at the route's auto-scope (we deliberately don't pass `scope` so
  // the auto-scope hook follows the current view). When the chat
  // provider isn't mounted (unlikely outside tests), this no-ops so
  // the palette doesn't throw.
  const openChat = (prefill?: string) =>
    run(() => {
      chatUi?.open({ tab: "thread", prefill: prefill || undefined })
    })

  return (
    <>
      <CommandDialog open={open} onOpenChange={setOpen}>
        {/* shadcn's CommandDialog wraps in <DialogContent> but NOT in
            <Command>; the cmdk primitives below need a Command parent
            for context, otherwise CommandInput throws and the dialog
            tears down the tree (blank screen). */}
        <Command>
          <CommandInput
            placeholder="Type a command or search…"
            value={search}
            onValueChange={setSearch}
          />
          <CommandList>
          <CommandEmpty>No results.</CommandEmpty>

            {chatUi && (
              <CommandGroup heading="Chat">
                <CommandItem
                  value="ask"
                  keywords={["chat", "question", "talk", "assistant"]}
                  // forceMount when the user is typing an inline message
                  // — cmdk's default fuzzy filter would otherwise drop
                  // this item for queries like "ask: why did run #42
                  // fail?" that don't score against just "ask".
                  forceMount={hasAskGrammar ? true : undefined}
                  onSelect={() => openChat(askPrefill ?? undefined)}
                >
                  <MessageSquare className="mr-2 h-4 w-4" />
                  {showInlineLabel ? (
                    <span>
                      Ask:{" "}
                      <span className="font-medium text-foreground">
                        {askPrefill}
                      </span>
                    </span>
                  ) : (
                    "Ask the assistant"
                  )}
                </CommandItem>
              </CommandGroup>
            )}

            {chatUi && <CommandSeparator />}

            <CommandGroup heading="Create">
              <CommandItem
                keywords={["new", "factory", "github"]}
                onSelect={() => go(scoped("workflows/start"))}
              >
                <Zap className="mr-2 h-4 w-4" />
                New workflow
              </CommandItem>
              <CommandItem
                keywords={["new"]}
                onSelect={() => run(() => setSessionOpen(true))}
              >
                <Terminal className="mr-2 h-4 w-4" />
                New session
              </CommandItem>
              <CommandItem
                keywords={["new", "tenant"]}
                onSelect={() => run(() => setWorkspaceOpen(true))}
              >
                <Building2 className="mr-2 h-4 w-4" />
                New workspace
              </CommandItem>
            </CommandGroup>

            <CommandSeparator />

            <CommandGroup heading="Go to">
              <CommandItem onSelect={() => go(scoped("workflows"))}>
                <GitBranch className="mr-2 h-4 w-4" />
                Workflows
              </CommandItem>
              <CommandItem onSelect={() => go(scoped("runs"))}>
                <Activity className="mr-2 h-4 w-4" />
                Runs
              </CommandItem>
              <CommandItem onSelect={() => go(scoped("activity"))}>
                <Zap className="mr-2 h-4 w-4" />
                Activity
              </CommandItem>
              <CommandItem onSelect={() => go("/workspaces")}>
                <Building2 className="mr-2 h-4 w-4" />
                Workspaces
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
      <NewWorkspaceDialog
        open={workspaceOpen}
        onOpenChange={setWorkspaceOpen}
      />
    </>
  )
}
