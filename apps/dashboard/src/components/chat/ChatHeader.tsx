import { Pin, PinOff, X } from "lucide-react"
import { Button } from "@/components/ui/button"
import type { Workspace } from "@/lib/api"
import type { ChatScope } from "@/lib/chat/scope"
import { ChatBreadcrumb } from "./ChatBreadcrumb"

// The chat header. Renders the scope path as a breadcrumb (ADR 0020
// § Contextual auto-scope), the pin affordance (filled icon when
// pinned), and the close button.

interface ChatHeaderProps {
  workspace: Workspace | null
  scope: ChatScope
  isPinned: boolean
  onPinToggle: () => void
  onClose: () => void
}

export function ChatHeader({
  workspace,
  scope,
  isPinned,
  onPinToggle,
  onClose,
}: ChatHeaderProps) {
  return (
    <header className="flex shrink-0 items-center gap-2 border-b px-3 py-2">
      <div className="min-w-0 flex-1">
        {workspace ? (
          <ChatBreadcrumb workspace={workspace} scope={scope} />
        ) : (
          <span className="text-xs text-muted-foreground">No workspace</span>
        )}
      </div>
      <Button
        type="button"
        variant="ghost"
        size="icon"
        className="h-7 w-7"
        aria-pressed={isPinned}
        aria-label={isPinned ? "Unpin chat scope" : "Pin chat scope"}
        title={
          isPinned
            ? "Unpin — chat will follow navigation again"
            : "Pin — lock chat to the current scope across navigation"
        }
        onClick={onPinToggle}
        disabled={!workspace}
      >
        {isPinned ? (
          <Pin className="h-3.5 w-3.5 fill-current" />
        ) : (
          <PinOff className="h-3.5 w-3.5" />
        )}
      </Button>
      <Button
        type="button"
        variant="ghost"
        size="icon"
        className="h-7 w-7"
        aria-label="Close chat"
        onClick={onClose}
      >
        <X className="h-4 w-4" />
      </Button>
    </header>
  )
}
