import { Skeleton } from "@/components/ui/skeleton"
import { PageSkeleton } from "./PageSkeleton"

/**
 * Full-page shell placeholder rendered before auth resolves or while a
 * lazy route chunk is loading. Static — no hooks, no context, no
 * network. Mirrors the geometry of AppLayout (top chrome height, mobile
 * tabs row, main padding) so swapping to the real UI causes no layout
 * shift.
 */
export function AppShellSkeleton() {
  return (
    <div
      className="flex h-svh w-full flex-col overflow-hidden"
      aria-busy="true"
      aria-live="polite"
    >
      {/* Desktop top chrome */}
      <header className="hidden h-14 items-center gap-3 border-b px-6 md:flex">
        <Skeleton className="h-7 w-7 rounded-md" />
        <Skeleton className="h-5 w-24" />
        <Skeleton className="h-8 w-44 rounded-md" />
        <div className="flex gap-1">
          <Skeleton className="h-7 w-20 rounded-md" />
          <Skeleton className="h-7 w-16 rounded-md" />
          <Skeleton className="h-7 w-20 rounded-md" />
        </div>
        <div className="ml-auto flex items-center gap-2">
          <Skeleton className="h-9 w-9 rounded-full" />
          <Skeleton className="h-9 w-9 rounded-full" />
        </div>
      </header>
      {/* Mobile page-title bar */}
      <header className="sticky top-0 z-30 flex h-14 items-center gap-2 border-b bg-background/95 px-3 backdrop-blur md:hidden">
        <Skeleton className="h-9 w-9 rounded-md" />
        <Skeleton className="h-5 w-24" />
        <Skeleton className="ml-auto h-9 w-9 rounded-full" />
        <Skeleton className="h-9 w-9 rounded-full" />
      </header>
      {/* Mobile tabs row */}
      <nav className="flex items-center gap-2 border-b px-2 py-1 md:hidden">
        <Skeleton className="h-7 w-20 rounded-full" />
        <Skeleton className="h-7 w-16 rounded-full" />
        <Skeleton className="h-7 w-20 rounded-full" />
      </nav>
      <main className="min-h-0 flex-1 overflow-y-auto p-4 pb-[calc(env(safe-area-inset-bottom)+1rem)] md:p-6 md:pb-6">
        <PageSkeleton />
      </main>
    </div>
  )
}
