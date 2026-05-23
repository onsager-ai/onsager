import { useCallback, useState } from "react"
import { Link, useNavigate } from "react-router-dom"
import { ArrowRight, FileText, Plus, Trash2 } from "lucide-react"
import { Button } from "@/components/ui/button"
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card"
import { useAuth } from "@/lib/auth"
import { useWorkflowDraft } from "@/lib/drafts"
import { useOSSFlag } from "@/hooks/useOSSFlag"
import type { WorkflowDraft } from "@/components/workflows/workflow-draft"

// Drafted-chip view (#467, ADR 0019 + 0020). Replaces the chat-page
// `DraftStrip` as the surface where users see, switch between, and
// continue editing their client-side drafts. Drafts live in
// localStorage per spec #401; the rendering here mirrors that shape
// — newest first, quick "Continue" entry per draft, "+ New draft"
// affordance, OSS "drafts on this device" footer.
//
// "Continue in chat" routes to `/chat?draft=<id>`. The route bounces
// to the workspace overview today (chat is mid-migration to a global
// component per ADR 0020 / spec #451) but the URL is the forward-
// compatible target — once ChatContainer lands, the param wakes the
// global surface on the right draft.
export function LocalDraftsCard() {
  const { user } = useAuth()
  const isOss = useOSSFlag()
  const navigate = useNavigate()
  const { drafts, newDraft, deleteById } = useWorkflowDraft(user?.id ?? null)

  const handleNew = useCallback(() => {
    const fresh = newDraft()
    navigate(`/chat?draft=${encodeURIComponent(fresh.id)}`)
  }, [newDraft, navigate])

  return (
    <Card>
      <CardHeader className="flex flex-row items-center justify-between gap-2 px-4 pb-2 pt-4 md:px-6">
        <CardTitle className="flex items-center gap-2 text-base">
          <FileText className="h-4 w-4 text-muted-foreground" />
          Local drafts
        </CardTitle>
        <Button size="sm" variant="outline" onClick={handleNew}>
          <Plus className="h-4 w-4" />
          New draft
        </Button>
      </CardHeader>
      <CardContent className="space-y-2 px-4 pb-4 md:px-6">
        {drafts.length === 0 ? (
          <p className="text-sm text-muted-foreground">
            No drafts on this device yet. Start one and the agent will
            help you shape it.
          </p>
        ) : (
          <ul className="space-y-2">
            {drafts.map((d) => (
              <DraftRow
                key={d.id}
                draft={d}
                canDelete={drafts.length > 1}
                onDelete={() => deleteById(d.id)}
              />
            ))}
          </ul>
        )}
        {isOss && drafts.length > 0 && (
          <p className="pt-1 text-[10px] text-muted-foreground/70">
            Drafts on this device.
          </p>
        )}
      </CardContent>
    </Card>
  )
}

interface DraftRowProps {
  draft: WorkflowDraft
  canDelete: boolean
  onDelete: () => void
}

function DraftRow({ draft, canDelete, onDelete }: DraftRowProps) {
  const [confirmingDelete, setConfirmingDelete] = useState(false)
  const stageCount = draft.workflow.stages.length
  const trigger = draft.workflow.trigger
  const hasRepo =
    trigger.repo_owner.trim() !== "" && trigger.repo_name.trim() !== ""
  const continueHref = `/chat?draft=${encodeURIComponent(draft.id)}`
  return (
    <li className="group flex items-center gap-3 rounded-md border bg-background px-3 py-2 hover:border-primary/40">
      <Link
        to={continueHref}
        className="min-w-0 flex-1 rounded-sm focus:outline-none focus-visible:ring-2 focus-visible:ring-ring"
        aria-label={`Continue draft ${draft.name || "Untitled draft"} in chat`}
      >
        <div className="truncate text-sm font-medium">
          {draft.name || "Untitled draft"}
        </div>
        <div className="truncate text-xs text-muted-foreground">
          {hasRepo
            ? `${trigger.repo_owner}/${trigger.repo_name}${trigger.label ? ` · ${trigger.label}` : ""}`
            : "No trigger yet"}
          {" · "}
          {stageCount === 0
            ? "0 stages"
            : `${stageCount} stage${stageCount === 1 ? "" : "s"}`}
          {" · "}
          {formatRelative(draft.updated_at)}
        </div>
      </Link>
      <Button size="sm" variant="ghost" render={<Link to={continueHref} />}>
        Continue
        <ArrowRight className="h-3.5 w-3.5" />
      </Button>
      {canDelete && (
        confirmingDelete ? (
          <Button
            size="sm"
            variant="destructive"
            onClick={() => {
              setConfirmingDelete(false)
              onDelete()
            }}
            aria-label={`Confirm delete ${draft.name || "draft"}`}
          >
            Delete?
          </Button>
        ) : (
          <Button
            size="icon-sm"
            variant="ghost"
            className="opacity-0 transition-opacity group-hover:opacity-70 hover:opacity-100 focus-visible:opacity-100"
            onClick={() => setConfirmingDelete(true)}
            aria-label={`Delete ${draft.name || "draft"}`}
          >
            <Trash2 className="h-3.5 w-3.5" />
          </Button>
        )
      )}
    </li>
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
