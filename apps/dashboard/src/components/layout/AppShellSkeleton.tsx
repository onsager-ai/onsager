import { Skeleton } from "@/components/ui/skeleton"
import { PageSkeleton } from "./PageSkeleton"

/**
 * Full-page shell placeholder rendered before auth resolves or while a
 * lazy route chunk is loading. Static — no hooks, no context, no
 * network. Mirrors the geometry of AppLayout (sidebar width, header
 * height, main padding) so swapping to the real UI causes no layout
 * shift.
 */
export function AppShellSkeleton() {
  return (
    <div className="flex min-h-screen w-full" aria-busy="true" aria-live="polite">
      {/* Sidebar — hidden on mobile to match SidebarProvider default */}
      <aside className="hidden w-64 shrink-0 border-r bg-sidebar md:flex md:flex-col">
        <div className="flex h-[60px] items-center gap-2 border-b px-6">
          <Skeleton className="h-6 w-6 rounded-md" />
          <Skeleton className="h-5 w-24" />
        </div>
        <div className="flex-1 space-y-6 px-3 py-4">
          {Array.from({ length: 4 }).map((_, s) => (
            <div key={s} className="space-y-2">
              <Skeleton className="mx-2 h-3 w-20" />
              {Array.from({ length: 2 }).map((_, i) => (
                <div key={i} className="flex items-center gap-2 rounded-md px-2 py-2">
                  <Skeleton className="h-4 w-4 rounded-sm" />
                  <Skeleton className="h-4 w-24" />
                </div>
              ))}
            </div>
          ))}
        </div>
        <div className="border-t p-4">
          <Skeleton className="h-3 w-12" />
        </div>
      </aside>

      <div className="flex min-w-0 flex-1 flex-col">
        {/* Mobile header */}
        <header className="sticky top-0 z-30 flex h-14 items-center gap-2 border-b bg-background/95 px-3 backdrop-blur md:hidden">
          <Skeleton className="h-9 w-9 rounded-md" />
          <Skeleton className="h-5 w-24" />
          <Skeleton className="ml-auto h-9 w-9 rounded-full" />
          <Skeleton className="h-9 w-9 rounded-full" />
        </header>
        {/* Desktop header */}
        <header className="hidden h-14 items-center gap-2 border-b px-6 md:flex">
          <Skeleton className="h-7 w-7 rounded-md" />
          <div className="ml-auto flex items-center gap-2">
            <Skeleton className="h-9 w-28 rounded-md" />
            <Skeleton className="h-9 w-9 rounded-full" />
          </div>
        </header>
        <main className="flex-1 p-4 pb-[calc(env(safe-area-inset-bottom)+1rem)] md:p-6 md:pb-6">
          <PageSkeleton />
        </main>
      </div>
    </div>
  )
}
