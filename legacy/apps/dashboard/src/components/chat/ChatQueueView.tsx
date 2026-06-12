import { CheckCircle2 } from "lucide-react"
import type { ChatScope } from "@/lib/chat/scope"

// Queue tab placeholder (ADR 0020 § Mount).
//
// The Queue is the HITL aggregator for the current scope — every
// pending decision (Approve/Reject, Selection, Clarify) that an agent
// has asked for in this scope renders as a card the user can act on
// inline. The aggregator wiring (subscribing to portal HITL events,
// rendering each card via the shared `HitlCard` primitive, action-
// button routing) lands as part of #472 (invocation channels +
// push-notification deep-link surface).
//
// This sub-issue (#471) lands the empty-state shape so the segmented
// control has both halves wired and the default-tab logic in
// `ChatContainer` has a real Queue tab to swap to.

interface ChatQueueViewProps {
  scope: ChatScope
  /**
   * Set by push-notification deep-links — once the HITL aggregator
   * lands, the matching card scrolls into view + highlights. Today the
   * queue is empty so the id is just rendered as a data attribute for
   * tests + the future aggregator to read.
   */
  highlightHitlId?: string | null
}

export function ChatQueueView({ scope, highlightHitlId }: ChatQueueViewProps) {
  return (
    <div
      data-slot="chat-queue-empty"
      data-scope={scope.type}
      data-highlight-hitl-id={highlightHitlId ?? undefined}
      className="flex flex-1 flex-col items-center justify-center gap-2 p-6 text-center text-sm text-muted-foreground"
    >
      <CheckCircle2 className="h-8 w-8 text-muted-foreground/50" />
      <div>No pending decisions in this scope.</div>
      <div className="text-xs">
        Agent-requested approvals will queue here.
      </div>
    </div>
  )
}
