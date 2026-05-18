import { Plus, X } from "lucide-react"
import { Button } from "@/components/ui/button"
import type { WorkflowDraft } from "@/components/factory/workflows/workflow-draft"

export interface DraftStripProps {
  drafts: WorkflowDraft[]
  activeId: string | null
  onSwitch: (id: string) => void
  onNew: () => void
  onDelete: (id: string) => void
}

// Spec #401's lightweight draft-quick-access strip. One chip per stored
// draft (`Untitled draft · 2h ago`), plus a `+ New draft` chip. Lives at
// the top of ChatPage; no dedicated drafts page in v1.
export function DraftStrip({
  drafts,
  activeId,
  onSwitch,
  onNew,
  onDelete,
}: DraftStripProps) {
  if (drafts.length === 0) return null
  return (
    <div className="flex shrink-0 items-center gap-1.5 overflow-x-auto border-b px-3 py-1.5">
      {drafts.map((d) => {
        const isActive = d.id === activeId
        return (
          <div
            key={d.id}
            className={
              "group inline-flex shrink-0 items-center gap-1 rounded-full border px-2 py-1 text-xs " +
              (isActive
                ? "border-primary bg-primary/10 text-foreground"
                : "border-transparent bg-muted/40 text-muted-foreground hover:bg-muted")
            }
          >
            <button
              type="button"
              onClick={() => onSwitch(d.id)}
              className="max-w-[200px] truncate"
              aria-current={isActive ? "true" : undefined}
            >
              {d.name || "Untitled draft"}
              <span className="ml-1 opacity-60">· {formatRelative(d.updated_at)}</span>
            </button>
            {drafts.length > 1 && (
              <button
                type="button"
                aria-label={`Delete ${d.name || "draft"}`}
                onClick={() => onDelete(d.id)}
                className="opacity-0 transition-opacity group-hover:opacity-70 hover:opacity-100"
              >
                <X className="h-3 w-3" />
              </button>
            )}
          </div>
        )
      })}
      <Button
        type="button"
        size="sm"
        variant="ghost"
        className="h-6 shrink-0 rounded-full px-2 text-xs"
        onClick={onNew}
        aria-label="New draft"
      >
        <Plus className="mr-1 h-3 w-3" />
        New draft
      </Button>
    </div>
  )
}

function formatRelative(iso: string): string {
  const t = Date.parse(iso)
  if (Number.isNaN(t)) return ""
  const diffMs = Date.now() - t
  const sec = Math.max(0, Math.round(diffMs / 1000))
  if (sec < 60) return `${sec}s ago`
  const min = Math.round(sec / 60)
  if (min < 60) return `${min}m ago`
  const hr = Math.round(min / 60)
  if (hr < 24) return `${hr}h ago`
  const day = Math.round(hr / 24)
  return `${day}d ago`
}
