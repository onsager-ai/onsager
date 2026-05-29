import type { ReactNode } from "react"
import { ArrowLeft } from "lucide-react"
import { Link, useLocation } from "react-router-dom"
import { Button } from "@/components/ui/button"
import { Separator } from "@/components/ui/separator"
import { OnsagerLogo } from "./OnsagerLogo"
import {
  CommandPaletteProvider,
  CommandPaletteTrigger,
} from "./CommandPalette"
import {
  PageHeaderProvider,
  usePageHeaderState,
} from "./PageHeader"
import { UserMenu } from "./UserMenu"
import { WorkspaceSwitcher } from "@/components/workspaces/WorkspaceSwitcher"
import { useOptionalActiveWorkspace } from "@/lib/workspace"
import { ChatUiProvider } from "@/lib/chat/use-chat-ui"
import { ChatContainer } from "@/components/chat/ChatContainer"
import { ChatFab } from "@/components/chat/ChatFab"
import { ChatBell } from "@/components/chat/ChatBell"
import { ChatDeepLinkHandler } from "@/components/chat/ChatDeepLinkHandler"
import { cn } from "@/lib/utils"

// Top chrome IA (#453 / ADR 0019). Three tabs — Workflows / Runs /
// Activity — anchor every workspace surface. The workspace switcher
// and the avatar/user menu sit in the same row; on narrow viewports
// the row compresses to a second row that holds the tabs.

interface TopTab {
  key: string
  label: string
  // Path relative to the active workspace root.
  path: string
}

const TOP_TABS: TopTab[] = [
  { key: "workflows", label: "Workflows", path: "workflows" },
  { key: "runs", label: "Runs", path: "runs" },
  { key: "activity", label: "Activity", path: "activity" },
]

export function AppLayout({ children }: { children: ReactNode }) {
  // ChatUi is mounted *above* CommandPalette so the palette's chat
  // command (⌘K → `ask: …`, spec #473) can call `useChatUi.open()`.
  // CommandPalette's chat command uses `useOptionalChatUi` so other
  // mounts (tests, isolated stories) still render without throwing.
  return (
    <PageHeaderProvider>
      <ChatUiProvider>
        <CommandPaletteProvider>
          <AppLayoutInner>{children}</AppLayoutInner>
        </CommandPaletteProvider>
      </ChatUiProvider>
    </PageHeaderProvider>
  )
}

function AppLayoutInner({ children }: { children: ReactNode }) {
  const { fullBleed } = usePageHeaderState()
  return (
    // Pin the shell to the viewport so the inner <main> owns scroll;
    // html/body are overflow:hidden in index.css.
    <div className="flex h-svh flex-col overflow-hidden">
      <DesktopTopChrome />
      <MobileHeader />
      <MobileTabsRow />
      <main
        className={
          fullBleed
            ? "min-h-0 flex-1 overflow-hidden"
            : "min-h-0 flex-1 overflow-y-auto overscroll-contain p-4 pb-[calc(env(safe-area-inset-bottom)+1rem)] md:p-6 md:pb-6"
        }
      >
        {children}
      </main>
      {/* Global chat surface (ADR 0020 / spec #471). One responsive
          mount across mobile (bottom-anchored overlay) and desktop
          (right-anchored 420px sidebar). Closed by default; ⌘J, the
          mobile FAB, the top-chrome bell, or scope-local Ask buttons
          bring it up (#472). */}
      <ChatContainer />
      {/* Mobile-only FAB — three-state opener pinned to the bottom
          right. Hidden on desktop (bell + ⌘J cover that surface). */}
      <ChatFab />
      {/* Deep-link handler for push-notification clicks. Reads the URL
          params and opens the chat at the carried scope / HITL id. */}
      <ChatDeepLinkHandler />
    </div>
  )
}

function DesktopTopChrome() {
  const activeWorkspace = useOptionalActiveWorkspace()
  const overviewPath = activeWorkspace
    ? `/workspaces/${activeWorkspace.slug}/workflows`
    : "/workspaces"
  return (
    <header className="hidden h-14 items-center gap-3 border-b px-4 md:flex md:px-6">
      <Link
        to={overviewPath}
        className="flex shrink-0 items-center gap-2"
        aria-label="Onsager home"
      >
        <OnsagerLogo size={22} />
        <span className="text-base font-semibold">Onsager</span>
      </Link>
      <Separator orientation="vertical" className="h-6" />
      <WorkspaceSwitcher />
      <Separator orientation="vertical" className="h-6" />
      <TopTabs />
      <div className="ml-auto flex items-center gap-1">
        <ChatBell />
        <CommandPaletteTrigger />
        <UserMenu variant="icon" />
      </div>
    </header>
  )
}

function TopTabs() {
  const activeWorkspace = useOptionalActiveWorkspace()
  const location = useLocation()
  const linkBase = activeWorkspace
    ? `/workspaces/${activeWorkspace.slug}`
    : null
  return (
    <nav className="flex items-center gap-1" aria-label="Workspace sections">
      {TOP_TABS.map((tab) => {
        const path = linkBase ? `${linkBase}/${tab.path}` : "/workspaces"
        const isActive =
          linkBase != null &&
          (location.pathname === path ||
            location.pathname.startsWith(`${path}/`))
        return (
          <Button
            key={tab.key}
            variant={isActive ? "default" : "ghost"}
            size="sm"
            className={cn(
              "h-8 px-3 text-sm",
              !isActive && "text-muted-foreground hover:text-foreground",
            )}
            render={<Link to={path} />}
          >
            {tab.label}
          </Button>
        )
      })}
    </nav>
  )
}

function MobileTabsRow() {
  const activeWorkspace = useOptionalActiveWorkspace()
  const location = useLocation()
  const linkBase = activeWorkspace
    ? `/workspaces/${activeWorkspace.slug}`
    : null
  // The mobile top bar already owns the page-title + back arrow row;
  // this second row holds the three workspace tabs so the chrome
  // matches the desktop IA.
  return (
    <nav
      className="flex items-center gap-1 overflow-x-auto border-b px-2 py-1 md:hidden"
      aria-label="Workspace sections (mobile)"
    >
      {TOP_TABS.map((tab) => {
        const path = linkBase ? `${linkBase}/${tab.path}` : "/workspaces"
        const isActive =
          linkBase != null &&
          (location.pathname === path ||
            location.pathname.startsWith(`${path}/`))
        return (
          <Button
            key={tab.key}
            variant={isActive ? "default" : "ghost"}
            size="sm"
            className={cn(
              "h-7 shrink-0 rounded-full px-3 text-xs",
              !isActive && "text-muted-foreground",
            )}
            render={<Link to={path} />}
          >
            {tab.label}
          </Button>
        )
      })}
    </nav>
  )
}

// Mobile chrome — the page-title bar on phones. Pages register
// title/back/actions via usePageHeader. When a page hasn't registered
// a title, fall back to the Onsager wordmark so the bar isn't empty.
function MobileHeader() {
  const { title, backTo, actions } = usePageHeaderState()

  return (
    <header className="sticky top-0 z-30 flex h-14 items-center gap-2 border-b bg-background/95 px-2 backdrop-blur supports-backdrop-filter:bg-background/80 md:hidden">
      {backTo ? (
        <Button
          variant="ghost"
          size="icon"
          className="h-9 w-9"
          aria-label="Go back"
          render={<Link to={backTo} />}
        >
          <ArrowLeft className="h-5 w-5" />
        </Button>
      ) : (
        <Link to="/" className="flex items-center gap-2 pl-1">
          <OnsagerLogo size={20} />
          <span className="text-base font-semibold">Onsager</span>
        </Link>
      )}
      <div className="min-w-0 flex-1">
        {title && (
          <div className="truncate text-base font-semibold">{title}</div>
        )}
      </div>
      <div className="flex items-center gap-1">
        {actions}
        <ChatBell size="sm" />
        <CommandPaletteTrigger />
        <UserMenu variant="icon" />
      </div>
    </header>
  )
}
