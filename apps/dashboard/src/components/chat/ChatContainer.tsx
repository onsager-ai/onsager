import { useEffect, useMemo, useRef } from "react"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { useAuth } from "@/lib/auth"
import { useOptionalActiveWorkspace } from "@/lib/workspace"
import { useChatUi } from "@/lib/chat/use-chat-ui"
import { useAutoScope } from "@/lib/chat/use-auto-scope"
import { useHitlCount } from "@/lib/chat/use-hitl-count"
import { ChatHeader } from "./ChatHeader"
import { ChatThreadView } from "./ChatThreadView"
import { ChatQueueView } from "./ChatQueueView"
import { cn } from "@/lib/utils"

// ChatContainer (ADR 0020 § Mount, spec #471).
//
// Mounts at the app-shell root (next to `<main>`) so the chat is
// reachable from every authenticated route. Closed by default; the
// invocation channels (#472 FAB / bell / Ask, #473 ⌘K) open it.
//
// Two viewports, one component:
//   Mobile  — bottom-anchored overlay, ~70vh, full width, drag-down
//             or close-button dismiss
//   Desktop — right-anchored sidebar, 420px fixed, full height,
//             ⌘J or close-button dismiss
//
// No user-selectable mount form. The ⌘J binding lives in
// `useChatUi`; this component renders the surface and routes the
// scope/tab state.

export function ChatContainer() {
  const { isOpen, close, tab, setTab, scopeOverride, highlightHitlId } = useChatUi()
  const { user } = useAuth()
  const workspace = useOptionalActiveWorkspace()
  const userId = user?.id ?? null
  const workspaceId = workspace?.id ?? null

  const autoScopeState = useAutoScope(userId, workspaceId, scopeOverride)
  const { effectiveScope, isPinned, pin, unpin } = autoScopeState
  const scopeHitlCount = useHitlCount(effectiveScope)

  // Default-tab logic per ADR 0020: Queue when the current scope has
  // pending HITL, Thread otherwise. The rule re-applies when the user
  // *navigates while the chat is open*, not when the chat first opens —
  // open() callers (FAB / bell / Ask / deep-link / ⌘J) own the
  // initial tab. The `wasOpenRef` gate distinguishes "just opened"
  // (caller picked the tab) from "scope changed mid-session" (we
  // re-default).
  const seenScopeRef = useRef<string | null>(null)
  const wasOpenRef = useRef(false)
  useEffect(() => {
    const wasOpen = wasOpenRef.current
    wasOpenRef.current = isOpen
    if (!isOpen) {
      seenScopeRef.current = null
      return
    }
    const key = `${effectiveScope.type}:${effectiveScope.id ?? ""}`
    if (!wasOpen) {
      // Just opened — record the scope but leave the tab the caller
      // picked alone.
      seenScopeRef.current = key
      return
    }
    if (seenScopeRef.current === key) return
    seenScopeRef.current = key
    setTab(scopeHitlCount > 0 ? "queue" : "thread")
  }, [isOpen, effectiveScope, scopeHitlCount, setTab])

  // Body-scroll lock on mobile when open — without it, swiping on the
  // overlay can bleed into the underlying page scroll. The app shell
  // already pins html/body to overflow-hidden (see index.css), but the
  // overlay itself owns scroll; locking the touch listener at the
  // backdrop layer is enough.
  const onBackdropClick = () => close()

  const tabsValue = useMemo(() => tab, [tab])

  // Render nothing when closed so the offscreen tree doesn't keep
  // React Queries warm. Re-opens re-mount the thread query — fine,
  // since the staleTime keeps the cache hit immediate.
  if (!isOpen) return null

  return (
    <>
      {/* Backdrop — mobile-only. Desktop overlays the right gutter
          and the user can keep interacting with the page behind. */}
      <div
        data-slot="chat-backdrop"
        className="fixed inset-0 z-40 bg-black/30 md:hidden"
        aria-hidden="true"
        onClick={onBackdropClick}
      />
      <aside
        data-slot="chat-container"
        role="dialog"
        aria-label="Onsager chat"
        className={cn(
          "fixed z-50 flex flex-col bg-background shadow-xl",
          // Mobile: bottom-anchored, ~70vh, full width
          "inset-x-0 bottom-0 h-[70vh] rounded-t-xl border-t",
          // Desktop: right-anchored, 420px, full height
          "md:inset-y-0 md:right-0 md:left-auto md:h-svh md:w-[420px] md:rounded-none md:border-l md:border-t-0",
        )}
      >
        <ChatHeader
          workspace={workspace}
          scope={effectiveScope}
          isPinned={isPinned}
          onPinToggle={() => (isPinned ? unpin() : pin())}
          onClose={close}
        />
        <Tabs
          value={tabsValue}
          onValueChange={(v) => setTab(v as "queue" | "thread")}
          className="flex min-h-0 flex-1 flex-col gap-0"
        >
          <div className="shrink-0 border-b px-3 py-1.5">
            <TabsList className="w-full">
              <TabsTrigger value="queue" className="flex-1 text-xs">
                Queue
              </TabsTrigger>
              <TabsTrigger value="thread" className="flex-1 text-xs">
                Thread
              </TabsTrigger>
            </TabsList>
          </div>
          {/* Panels: base-ui sets `hidden` on the inactive panel; we
              render both but only the active one is visible. The
              `min-h-0 flex-1` and `flex flex-col` wrap each panel so
              the inner Queue/Thread views can size against the
              container. We don't put `flex` on TabsContent itself —
              that would override the `display: none` from `hidden`
              and both panels would stack. */}
          <TabsContent value="queue" className="min-h-0 flex-1">
            <div className="flex min-h-0 h-full flex-col">
              <ChatQueueView
                scope={effectiveScope}
                highlightHitlId={highlightHitlId}
              />
            </div>
          </TabsContent>
          {/* Thread panel is `keepMounted` so the conversation query
              warms in the background even when the user opens via the
              bell / push deep-link / FAB-with-pending (all default to
              Queue) — switching tabs then renders the messages
              immediately instead of waiting on a fresh fetch. */}
          <TabsContent value="thread" className="min-h-0 flex-1" keepMounted>
            <div className="flex min-h-0 h-full flex-col">
              {workspace ? (
                <ChatThreadView
                  workspace={workspace}
                  scope={effectiveScope}
                  userId={userId}
                />
              ) : (
                <NoWorkspaceState />
              )}
            </div>
          </TabsContent>
        </Tabs>
      </aside>
    </>
  )
}

function NoWorkspaceState() {
  return (
    <div className="flex flex-1 items-center justify-center p-6 text-center text-sm text-muted-foreground">
      Pick a workspace to start chatting.
    </div>
  )
}
