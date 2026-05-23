import { MessageSquare } from "lucide-react"
import { Button } from "@/components/ui/button"
import { useChatUi } from "@/lib/chat/use-chat-ui"
import type { ChatScope } from "@/lib/chat/scope"
import { cn } from "@/lib/utils"

// ChatAskButton (ADR 0020 § Invocation channels, spec #472).
//
// Small "Ask" button on step / verdict / artifact detail surfaces.
// Opens the global chat scoped to the object the user is looking at,
// default tab `Thread` (per the ADR — Ask is for conversational
// inquiry, not picking from a queue).
//
// The scope is passed explicitly because the auto-scope hook can't
// always derive the right object-level scope from the route alone
// (stages, verdicts inside runs, etc.). The button forwards the scope
// to `useChatUi.open({ scope })`, which the auto-scope hook honors as
// a one-shot override until the chat closes.

interface ChatAskButtonProps {
  /** Object-level scope. Workspace fallback is acceptable for artifacts. */
  scope: ChatScope
  /** Label override. Defaults to "Ask". */
  label?: string
  /** Visual variant — defaults to a small outline button. */
  variant?: "outline" | "ghost" | "default"
  size?: "sm" | "icon"
  className?: string
}

export function ChatAskButton({
  scope,
  label = "Ask",
  variant = "outline",
  size = "sm",
  className,
}: ChatAskButtonProps) {
  const { open } = useChatUi()
  const isIcon = size === "icon"
  return (
    <Button
      type="button"
      variant={variant}
      size={size}
      data-slot="chat-ask-button"
      data-scope-type={scope.type}
      aria-label={isIcon ? `${label} chat about this ${scope.type}` : undefined}
      onClick={() => open({ scope, tab: "thread" })}
      className={cn(isIcon && "h-8 w-8", className)}
    >
      <MessageSquare className={cn("h-3.5 w-3.5", !isIcon && "mr-1")} />
      {!isIcon && label}
    </Button>
  )
}
