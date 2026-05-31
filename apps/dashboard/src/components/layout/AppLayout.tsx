import { useState } from "react"
import type { ComponentType, ReactNode } from "react"
import {
  Activity,
  ArrowLeft,
  Menu,
  Play,
  Rocket,
  Settings,
  Workflow,
} from "lucide-react"
import { Link, useLocation } from "react-router-dom"
import { Button, buttonVariants } from "@/components/ui/button"
import { Separator } from "@/components/ui/separator"
import { Sheet, SheetContent, SheetTrigger } from "@/components/ui/sheet"
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
import { DesktopChatEntry } from "@/components/chat/DesktopChatEntry"
import { ChatDeepLinkHandler } from "@/components/chat/ChatDeepLinkHandler"
import { cn } from "@/lib/utils"

// Nav IA — PROTOTYPE (sidebar on desktop, slide-in drawer on mobile).
// Supersedes the top-bar-with-tabs chrome (#453 / ADR 0019) for the
// reasons in #517: the four section tabs could never express Settings /
// Workspaces, so those pages showed no active item. One sidebar body
// (`SidebarBody`) renders both as a persistent rail on desktop and the
// contents of a left off-canvas drawer on mobile (hamburger in the top
// bar). Every destination gets a first-class home with unambiguous
// active state. Final shape pending the spec/ADR we agreed to write.

interface NavSection {
  key: string
  label: string
  icon: ComponentType<{ className?: string }>
  // Path relative to the active workspace root.
  path: string
}

const NAV_SECTIONS: NavSection[] = [
  { key: "workflows", label: "Workflows", icon: Workflow, path: "workflows" },
  { key: "runs", label: "Runs", icon: Play, path: "runs" },
  // "Plans" is the noun-surface home for persisted spec plans (ADR 0023
  // / 0025, spec #514) — the launch surface that stops chat being the
  // sole spec-plan-run path. It is a sanctioned 5th nav noun; see the
  // vocabulary carve-out in the root CLAUDE.md.
  { key: "spec-plans", label: "Plans", icon: Rocket, path: "spec-plans" },
  { key: "activity", label: "Activity", icon: Activity, path: "activity" },
]

function isActivePath(pathname: string, target: string): boolean {
  return pathname === target || pathname.startsWith(`${target}/`)
}

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
    // html/body are overflow:hidden in index.css. Desktop is a sidebar +
    // content row; on mobile the rail collapses into the drawer and the
    // content column fills the width.
    <div className="flex h-svh overflow-hidden">
      <DesktopSidebar />
      <div className="flex min-w-0 flex-1 flex-col overflow-hidden">
        <MobileHeader />
        <main
          className={
            fullBleed
              ? "min-h-0 flex-1 overflow-hidden"
              : "min-h-0 flex-1 overflow-y-auto overscroll-contain p-4 pb-[calc(env(safe-area-inset-bottom)+1rem)] md:p-6 md:pb-6"
          }
        >
          {children}
        </main>
      </div>
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

// The sidebar contents — shared verbatim between the desktop rail and
// the mobile drawer. The parent (an <aside> or a <SheetContent>) owns
// the column sizing; this renders the logo, workspace switcher, section
// links, the Settings entry, and the account/chat footer cluster.
function SidebarBody({ onNavigate }: { onNavigate?: () => void }) {
  const activeWorkspace = useOptionalActiveWorkspace()
  const location = useLocation()
  const linkBase = activeWorkspace
    ? `/workspaces/${activeWorkspace.slug}`
    : null
  const overviewPath = linkBase ? `${linkBase}/workflows` : "/workspaces"
  // Settings lives in the workspace when we're in one, else the global
  // account settings. Either way it gets a first-class, highlightable
  // home in the footer — the gap the old top-tab IA left open.
  const settingsPath = linkBase ? `${linkBase}/settings` : "/settings"
  const settingsActive =
    location.pathname === "/settings" ||
    location.pathname.endsWith("/settings")

  return (
    <>
      <div className="flex h-14 shrink-0 items-center px-4">
        <Link
          to={overviewPath}
          onClick={onNavigate}
          className="flex items-center gap-2"
          aria-label="Onsager home"
        >
          <OnsagerLogo size={22} />
          <span className="text-base font-semibold">Onsager</span>
        </Link>
      </div>
      <div className="px-3 pb-3">
        <WorkspaceSwitcher />
      </div>
      <Separator />
      <nav
        className="flex-1 space-y-1 overflow-y-auto px-3 py-3"
        aria-label="Workspace sections"
      >
        {NAV_SECTIONS.map((section) => {
          const path = linkBase ? `${linkBase}/${section.path}` : "/workspaces"
          const active = linkBase != null && isActivePath(location.pathname, path)
          const Icon = section.icon
          return (
            <Link
              key={section.key}
              to={path}
              onClick={onNavigate}
              aria-current={active ? "page" : undefined}
              className={cn(
                buttonVariants({
                  variant: active ? "default" : "ghost",
                  size: "sm",
                }),
                "w-full justify-start gap-2",
                !active && "text-muted-foreground hover:text-foreground",
              )}
            >
              <Icon className="h-4 w-4" />
              {section.label}
            </Link>
          )
        })}
      </nav>
      <Separator />
      <div className="space-y-2 px-3 py-3">
        <Link
          to={settingsPath}
          onClick={onNavigate}
          aria-current={settingsActive ? "page" : undefined}
          className={cn(
            buttonVariants({
              variant: settingsActive ? "default" : "ghost",
              size: "sm",
            }),
            "w-full justify-start gap-2",
            !settingsActive && "text-muted-foreground hover:text-foreground",
          )}
        >
          <Settings className="h-4 w-4" />
          Settings
        </Link>
        <div className="flex items-center gap-1">
          <DesktopChatEntry />
          <ChatBell />
          <CommandPaletteTrigger />
          <div className="ml-auto">
            <UserMenu variant="icon" />
          </div>
        </div>
      </div>
    </>
  )
}

function DesktopSidebar() {
  return (
    <aside className="hidden w-60 shrink-0 flex-col border-r bg-background md:flex">
      <SidebarBody />
    </aside>
  )
}

// Mobile nav drawer — a left off-canvas Sheet holding the same sidebar
// body. Triggered by the hamburger in the top bar; closes itself on any
// route change (covers section links, the Settings entry, and workspace
// switches alike) and on backdrop tap.
function MobileNavDrawer() {
  const [open, setOpen] = useState(false)
  // Selecting a destination closes the drawer (handled per-link via
  // onNavigate); backdrop tap closes it the usual way.
  return (
    <Sheet open={open} onOpenChange={setOpen}>
      <SheetTrigger
        render={
          <Button
            variant="ghost"
            size="icon"
            className="h-9 w-9"
            aria-label="Open navigation"
          />
        }
      >
        <Menu className="h-5 w-5" />
      </SheetTrigger>
      <SheetContent
        side="left"
        showCloseButton={false}
        className="flex w-72 flex-col gap-0 p-0"
      >
        <SidebarBody onNavigate={() => setOpen(false)} />
      </SheetContent>
    </Sheet>
  )
}

// Mobile chrome — the page-title bar on phones. Pages register
// title/back/actions via usePageHeader. Top-level surfaces show the
// hamburger (opens the nav drawer); drill-down pages (those that set
// `backTo`) show a back arrow instead. When no title is registered we
// fall back to the Onsager wordmark so the bar isn't empty.
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
        <MobileNavDrawer />
      )}
      <div className="min-w-0 flex-1">
        {title ? (
          <div className="truncate text-base font-semibold">{title}</div>
        ) : (
          <span className="text-base font-semibold">Onsager</span>
        )}
      </div>
      {/* Account lives once in the drawer footer (open via ☰), per the
          dashboard-ui skill — not duplicated here. HITL on mobile is the
          chat FAB. The bar keeps only page actions + the ⌘K trigger. */}
      <div className="flex items-center gap-1">
        {actions}
        <CommandPaletteTrigger />
      </div>
    </header>
  )
}
