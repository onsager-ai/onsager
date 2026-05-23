import { useMemo, useState } from "react"
import { ChevronDown, ChevronRight, X } from "lucide-react"
import { Button } from "@/components/ui/button"
import {
  dismissLegacyLink,
  readLegacyConversations,
} from "@/lib/chat/legacy-localstorage"

// "Previous conversations" affordance (ADR 0020 § Storage schema
// change → fresh-start migration). One release window: legacy
// `localStorage` chat history is surfaced once inside the workspace-
// scoped Thread as a collapsible block. The user can read or copy
// the text out manually; nothing is written back to the server.

interface PreviousConversationsLinkProps {
  userId: string | null
}

export function PreviousConversationsLink({
  userId,
}: PreviousConversationsLinkProps) {
  const [expanded, setExpanded] = useState(false)
  const [dismissed, setDismissed] = useState(false)
  const conversations = useMemo(
    () => readLegacyConversations(userId),
    [userId],
  )

  if (dismissed || conversations.length === 0) return null

  return (
    <div className="mb-3 rounded-md border bg-muted/20 px-2.5 py-1.5 text-xs">
      <div className="flex items-center gap-2">
        <Button
          type="button"
          variant="ghost"
          size="sm"
          className="h-6 gap-1 px-1 text-xs"
          onClick={() => setExpanded((v) => !v)}
          aria-expanded={expanded}
        >
          {expanded ? (
            <ChevronDown className="h-3 w-3" />
          ) : (
            <ChevronRight className="h-3 w-3" />
          )}
          Previous conversations ({conversations.length})
        </Button>
        <Button
          type="button"
          variant="ghost"
          size="icon"
          className="ml-auto h-6 w-6"
          aria-label="Dismiss previous conversations"
          onClick={() => {
            setDismissed(true)
            dismissLegacyLink()
          }}
        >
          <X className="h-3 w-3" />
        </Button>
      </div>
      {expanded ? (
        <ol className="mt-2 flex flex-col gap-1.5 pl-4 text-muted-foreground">
          {conversations.map((c) => (
            <li key={c.storageKey}>
              <div className="font-mono text-[10px]">{c.workspaceIdHint}</div>
              <ol className="mt-1 list-decimal pl-4">
                {c.turns.slice(0, 5).map((t) => (
                  <li key={t.id} className="truncate">
                    {t.userContent}
                  </li>
                ))}
                {c.turns.length > 5 ? (
                  <li>… {c.turns.length - 5} more</li>
                ) : null}
              </ol>
            </li>
          ))}
        </ol>
      ) : null}
    </div>
  )
}
