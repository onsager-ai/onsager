import { useEffect, useMemo, useRef } from "react"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { useAuth } from "@/lib/auth"
import { useOptionalActiveWorkspace } from "@/lib/workspace"
import { useChatUi } from "@/lib/chat/use-chat-ui"
import { useAutoScope } from "@/lib/chat/use-auto-scope"
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
  const { isOpen, close, tab, setTab } = useChatUi()
  const { user } = useAuth()
  const workspace = useOptionalActiveWorkspace()
  const userId = user?.id ?? null
  const workspaceId = workspace?.id ?? null

  const autoScopeState = useAutoScope(userId, workspaceId)
  const { effectiveScope, isPinned, pin, unpin } = autoScopeState

  // Default-tab logic. The ADR's table says Queue when the scope has
  // pending HITL, Thread otherwise. HITL aggregation lands in #472, so
  // for now the default is always Thread; setting the tab via
  // `open({ tab: "queue" })` (e.g. from the bell once it lands) still
  // works. We expose the seam here so #472 can flip the rule by
  // wiring in a `hasPendingHitl(scope)` check.
  const seenScopeRef = useRef<string | null>(null)
  useEffect(() => {
    if (!isOpen) return
    const key = `${effectiveScope.type}:${effectiveScope.id ?? ""}`
    if (seenScopeRef.current === key) return
    seenScopeRef.current = key
    // When the open chat re-scopes (e.g. navigation while pinned-off),
    // reset to the natural default. Invocation channels can override
    // this by passing `{ tab }` to `open()` — see `useChatUi`.
    setTab("thread")
  }, [isOpen, effectiveScope, setTab])

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
              <ChatQueueView scope={effectiveScope} />
            </div>
          </TabsContent>
          <TabsContent value="thread" className="min-h-0 flex-1">
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
