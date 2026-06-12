import { Skeleton } from "@/components/ui/skeleton"
import { cn } from "@/lib/utils"

type Variant = "list" | "detail" | "default"

export function PageSkeleton({
  variant = "default",
  className,
}: {
  variant?: Variant
  className?: string
}) {
  if (variant === "detail") {
    return <DetailSkeleton className={className} />
  }
  if (variant === "list") {
    return <ListSkeleton className={className} />
  }
  return <DefaultSkeleton className={className} />
}

function PageHeaderSkeleton() {
  return (
    <div className="space-y-2">
      <Skeleton className="h-8 w-48" />
      <Skeleton className="h-4 w-72" />
    </div>
  )
}

function ListSkeleton({ className }: { className?: string }) {
  return (
    <div className={cn("space-y-6", className)} aria-busy="true" aria-live="polite">
      <PageHeaderSkeleton />
      <div className="rounded-lg border">
        <div className="flex items-center gap-4 border-b px-4 py-3">
          <Skeleton className="h-4 w-24" />
          <Skeleton className="h-4 w-32" />
          <Skeleton className="h-4 w-16" />
          <Skeleton className="ml-auto h-4 w-20" />
        </div>
        {Array.from({ length: 8 }).map((_, i) => (
          <div key={i} className="flex items-center gap-4 border-b px-4 py-4 last:border-b-0">
            <Skeleton className="h-4 w-24" />
            <Skeleton className="h-4 w-40" />
            <Skeleton className="h-5 w-16 rounded-full" />
            <Skeleton className="ml-auto h-4 w-24" />
          </div>
        ))}
      </div>
    </div>
  )
}

function DetailSkeleton({ className }: { className?: string }) {
  return (
    <div className={cn("space-y-6", className)} aria-busy="true" aria-live="polite">
      <PageHeaderSkeleton />
      <div className="grid gap-4 md:grid-cols-3">
        {Array.from({ length: 3 }).map((_, i) => (
          <div key={i} className="rounded-lg border p-4 space-y-3">
            <Skeleton className="h-4 w-20" />
            <Skeleton className="h-7 w-16" />
          </div>
        ))}
      </div>
      <div className="rounded-lg border p-6 space-y-3">
        <Skeleton className="h-5 w-32" />
        <Skeleton className="h-4 w-full" />
        <Skeleton className="h-4 w-11/12" />
        <Skeleton className="h-4 w-10/12" />
      </div>
    </div>
  )
}

function DefaultSkeleton({ className }: { className?: string }) {
  return (
    <div className={cn("space-y-6", className)} aria-busy="true" aria-live="polite">
      <PageHeaderSkeleton />
      <div className="grid gap-4 md:grid-cols-2 lg:grid-cols-4">
        {Array.from({ length: 4 }).map((_, i) => (
          <div key={i} className="rounded-lg border p-4 space-y-3">
            <Skeleton className="h-4 w-20" />
            <Skeleton className="h-7 w-24" />
          </div>
        ))}
      </div>
      <div className="rounded-lg border">
        <div className="border-b p-4">
          <Skeleton className="h-5 w-40" />
        </div>
        {Array.from({ length: 5 }).map((_, i) => (
          <div key={i} className="flex items-center gap-4 border-b p-4 last:border-b-0">
            <Skeleton className="h-9 w-9 rounded-full" />
            <div className="flex-1 space-y-2">
              <Skeleton className="h-4 w-1/3" />
              <Skeleton className="h-3 w-1/2" />
            </div>
            <Skeleton className="h-6 w-16 rounded-full" />
          </div>
        ))}
      </div>
    </div>
  )
}
