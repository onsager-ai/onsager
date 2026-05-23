import { Bell } from "lucide-react"
import { Button } from "@/components/ui/button"
import { useChatUi } from "@/lib/chat/use-chat-ui"
import { useWorkspaceHitlCount } from "@/lib/chat/use-hitl-count"
import { WORKSPACE_SCOPE } from "@/lib/chat/scope"
import { cn } from "@/lib/utils"

// ChatBell (ADR 0020 § Invocation channels, spec #472).
//
// Mounts in the top chrome (mobile and desktop). Opens the global chat
// at `scope=workspace`, default tab `Queue` — the Inbox. Counterpart
// to the mobile FAB; the bell is the always-available, scope-agnostic
// way in.
//
// The badge shows the count of pending HITL items across the whole
// workspace (workspace-scope count today; sums across finer scopes once
// the aggregator lands — see `use-hitl-count.ts`).

interface ChatBellProps {
  /** Compact variant for the mobile chrome row. */
  size?: "sm" | "md"
}

export function ChatBell({ size = "md" }: ChatBellProps) {
  const { open } = useChatUi()
  const count = useWorkspaceHitlCount()
  const isMd = size === "md"
  return (
    <Button
      type="button"
      variant="ghost"
      size="icon"
      className={cn("relative", isMd ? "h-9 w-9" : "h-8 w-8")}
      aria-label={
        count > 0 ? `Open Inbox — ${count} pending` : "Open Inbox"
      }
      onClick={() => open({ scope: WORKSPACE_SCOPE, tab: "queue" })}
      data-slot="chat-bell"
      data-count={count}
    >
      <Bell className={isMd ? "h-5 w-5" : "h-4 w-4"} />
      {count > 0 && (
        <span
          data-slot="chat-bell-badge"
          className={cn(
            "absolute flex items-center justify-center rounded-full bg-destructive text-destructive-foreground font-semibold",
            isMd
              ? "-right-0.5 -top-0.5 h-4 min-w-[1rem] px-1 text-[10px]"
              : "-right-0 -top-0 h-3.5 min-w-[0.875rem] px-1 text-[9px]",
          )}
        >
          {count > 99 ? "99+" : count}
        </span>
      )}
    </Button>
  )
}
