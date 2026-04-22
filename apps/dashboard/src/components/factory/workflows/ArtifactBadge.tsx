import { cn } from "@/lib/utils"
import type { WorkflowArtifactKind } from "@/lib/api"
import { artifactKindMeta } from "./workflow-meta"

export interface ArtifactBadgeProps {
  kind: WorkflowArtifactKind
  variant?: "default" | "muted"
  size?: "sm" | "md"
  className?: string
}

// Small pill that shows an artifact kind with an icon. Used everywhere
// stages or the trigger reference the artifact they flow — makes the
// "what kind of thing moves through here?" question visually obvious.
export function ArtifactBadge({
  kind,
  variant = "default",
  size = "sm",
  className,
}: ArtifactBadgeProps) {
  const meta = artifactKindMeta(kind)
  const Icon = meta.icon
  return (
    <span
      className={cn(
        "inline-flex items-center gap-1 rounded-full border font-medium",
        variant === "default"
          ? "border-primary/30 bg-primary/10 text-primary"
          : "border-muted-foreground/20 bg-muted text-muted-foreground",
        size === "sm" ? "px-2 py-0.5 text-xs" : "px-2.5 py-1 text-sm",
        className,
      )}
    >
      <Icon className={size === "sm" ? "h-3 w-3" : "h-3.5 w-3.5"} />
      {meta.shortLabel}
    </span>
  )
}
