import type { ReactNode } from "react"
import { ArrowLeft } from "lucide-react"
import { Link } from "react-router-dom"
import { SidebarProvider, SidebarInset, SidebarTrigger } from "@/components/ui/sidebar"
import { Button } from "@/components/ui/button"
import { Separator } from "@/components/ui/separator"
import { AppSidebar } from "./AppSidebar"
import { OnsagerLogo } from "./OnsagerLogo"
import { QuickCreateMenu } from "./QuickCreateMenu"
import {
  PageHeaderProvider,
  usePageHeaderState,
} from "./PageHeader"

export function AppLayout({ children }: { children: ReactNode }) {
  return (
    <PageHeaderProvider>
      <AppLayoutInner>{children}</AppLayoutInner>
    </PageHeaderProvider>
  )
}

function AppLayoutInner({ children }: { children: ReactNode }) {
  return (
    // Constrain the shell to the viewport so the inner <main> can be the
    // only scroll container — body is overflow:hidden in index.css.
    // shadcn's wrapper defaults to min-h-svh which would let content grow
    // and clip; h-svh + overflow-hidden pins it.
    <SidebarProvider className="h-svh overflow-hidden">
      <AppSidebar />
      <SidebarInset>
        <MobileHeader />
        {/* Desktop header */}
        <header className="hidden h-14 items-center gap-2 border-b px-6 md:flex">
          <SidebarTrigger />
          <Separator orientation="vertical" className="h-6" />
          <div className="ml-auto flex items-center gap-2">
            <QuickCreateMenu />
          </div>
        </header>
        {/* min-h-0 is required for flex-1 + overflow-y-auto to engage in a
            flex column: flex items default to min-height: auto and would
            otherwise grow to content height instead of scrolling. */}
        <main className="min-h-0 flex-1 overflow-y-auto overscroll-contain p-4 pb-[calc(env(safe-area-inset-bottom)+1rem)] md:p-6 md:pb-6">
          {children}
        </main>
      </SidebarInset>
    </SidebarProvider>
  )
}

// Mobile chrome — the only persistent bar on phones. Pages register
// title/back/actions via usePageHeader. When a page hasn't registered a
// title, we fall back to the Onsager wordmark to avoid an empty bar.
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
        <SidebarTrigger className="h-9 w-9" />
      )}
      <div className="min-w-0 flex-1">
        {title ? (
          <div className="truncate text-base font-semibold">{title}</div>
        ) : (
          <Link to="/" className="flex items-center gap-2">
            <OnsagerLogo size={20} />
            <span className="text-base font-semibold">Onsager</span>
          </Link>
        )}
      </div>
      {/* Page-specific actions, then global QuickCreate. UserMenu lives
          in the sidebar footer on mobile to reclaim header space. */}
      <div className="flex items-center gap-1">
        {actions}
        <QuickCreateMenu />
      </div>
    </header>
  )
}

