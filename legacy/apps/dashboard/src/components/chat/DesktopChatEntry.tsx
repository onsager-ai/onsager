import { MessageSquare } from "lucide-react"
import { Button } from "@/components/ui/button"
import { useChatUi } from "@/lib/chat/use-chat-ui"
import { WORKSPACE_SCOPE } from "@/lib/chat/scope"

// DesktopChatEntry (ADR 0025 § Desktop gains a labeled chat entry, spec
// #514).
//
// The labeled, general-purpose way *into a conversation* on desktop. ADR
// 0020 left desktop with only the bell (which opens the Inbox/Queue) and
// the bare `⌘J` toggle — no discoverable "start a conversation" affordance.
// This is it: a labeled button that opens the chat at the workspace scope
// on the Thread tab (the design/maintenance surface), distinct from the
// bell's "what needs my attention" Queue.
//
// Lives in the desktop top chrome only (its parent row is `md:flex`); the
// mobile FAB and the bell are unchanged.

export function DesktopChatEntry() {
  const { open } = useChatUi()
  return (
    <Button
      type="button"
      variant="ghost"
      size="sm"
      className="h-9 gap-1.5 px-2 text-muted-foreground hover:text-foreground"
      aria-label="Open chat"
      onClick={() => open({ scope: WORKSPACE_SCOPE, tab: "thread" })}
      data-slot="desktop-chat-entry"
    >
      <MessageSquare className="h-4 w-4" />
      <span className="text-sm">Chat</span>
      <kbd className="ml-0.5 hidden rounded border bg-muted px-1 py-0.5 font-mono text-[10px] text-muted-foreground lg:inline">
        ⌘J
      </kbd>
    </Button>
  )
}
