import type { ReactNode } from "react"
import { SidebarProvider, SidebarInset, SidebarTrigger } from "@/components/ui/sidebar"
import { AppSidebar } from "./AppSidebar"
import { Separator } from "@/components/ui/separator"
import { OnsagerLogo } from "./OnsagerLogo"
import { Link } from "react-router-dom"
import { UserMenu } from "./UserMenu"
import { QuickCreateMenu } from "./QuickCreateMenu"

export function AppLayout({ children }: { children: ReactNode }) {
  return (
    // Constrain the shell to the viewport so the inner <main> can be the
    // only scroll container — body is overflow:hidden in index.css.
    // shadcn's wrapper defaults to min-h-svh which would let content grow
    // and clip; h-svh + overflow-hidden pins it.
    <SidebarProvider className="h-svh overflow-hidden">
      <AppSidebar />
      <SidebarInset>
        {/* Mobile header */}
        <header className="sticky top-0 z-30 flex h-14 items-center gap-2 border-b bg-background/95 px-3 backdrop-blur supports-backdrop-filter:bg-background/80 md:hidden">
          <SidebarTrigger className="h-9 w-9" />
          <Link to="/" className="flex flex-1 items-center gap-2">
            <OnsagerLogo size={20} />
            <span className="text-base font-semibold">Onsager</span>
          </Link>
          <QuickCreateMenu />
          <UserMenu />
        </header>
        {/* Desktop header */}
        <header className="hidden h-14 items-center gap-2 border-b px-6 md:flex">
          <SidebarTrigger />
          <Separator orientation="vertical" className="h-6" />
          <div className="ml-auto flex items-center gap-2">
            <QuickCreateMenu />
            <UserMenu />
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
