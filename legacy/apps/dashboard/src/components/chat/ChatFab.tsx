import { useCallback, useEffect, useRef, useState } from "react"
import { MessageSquare } from "lucide-react"
import { cn } from "@/lib/utils"
import { useChatUi } from "@/lib/chat/use-chat-ui"
import { useAutoScope } from "@/lib/chat/use-auto-scope"
import { useHitlCount } from "@/lib/chat/use-hitl-count"
import { useAuth } from "@/lib/auth"
import { useOptionalActiveWorkspace } from "@/lib/workspace"
import { WORKSPACE_SCOPE } from "@/lib/chat/scope"

// ChatFab (ADR 0020 § FAB three-state behaviour, spec #472).
//
// Mobile-only floating action button. The desktop counterpart is the
// top-chrome bell + ⌘J. Three states driven by scope HITL state and
// viewport conditions, not user toggle:
//
//   Active — current scope has pending HITL → 52px filled circle,
//            accent badge with count, 3-second pulse ring on new HITL
//            arrival.
//   Muted  — current scope has no pending HITL → 40px outlined circle,
//            secondary icon, opacity 0.75.
//   Hidden — scroll-down velocity or virtual keyboard up → fade-out +
//            8px downward translate; restore on scroll-up or keyboard
//            dismiss.
//
// Long-press jumps to the Inbox (`scope=workspace`, Queue tab); a
// first-time hint surfaces on the third FAB use.

const LONG_PRESS_MS = 500
const LONG_PRESS_HAPTIC_MS = 20
// Scroll-velocity threshold (px/sec). Picked so a casual flick down
// hides the FAB but a slow read does not.
const HIDE_VELOCITY = 400
// The hint surfaces on the 3rd use per the ADR.
const HINT_USE_THRESHOLD = 3
const HINT_DURATION_MS = 4500

const FAB_USE_KEY = "onsager.chat.fab.useCount.v1"
const FAB_HINT_SEEN_KEY = "onsager.chat.fab.hintSeen.v1"

function readUseCount(): number {
  if (typeof localStorage === "undefined") return 0
  const raw = localStorage.getItem(FAB_USE_KEY)
  const n = raw ? parseInt(raw, 10) : 0
  return Number.isFinite(n) && n >= 0 ? n : 0
}

function writeUseCount(n: number) {
  try {
    localStorage.setItem(FAB_USE_KEY, String(n))
  } catch {
    /* ignore — quota / privacy mode */
  }
}

function markHintSeen() {
  try {
    localStorage.setItem(FAB_HINT_SEEN_KEY, "1")
  } catch {
    /* ignore */
  }
}

function hintAlreadySeen(): boolean {
  if (typeof localStorage === "undefined") return true
  return localStorage.getItem(FAB_HINT_SEEN_KEY) === "1"
}

interface ChatFabProps {
  /** Override visibility for tests + storybook. */
  forceVisible?: boolean
}

export function ChatFab({ forceVisible }: ChatFabProps = {}) {
  const { isOpen, open } = useChatUi()
  const { user } = useAuth()
  const workspace = useOptionalActiveWorkspace()
  const autoScope = useAutoScope(user?.id ?? null, workspace?.id ?? null)
  const pendingCount = useHitlCount(autoScope.effectiveScope)
  const [hidden, setHidden] = useState(false)
  const [hintVisible, setHintVisible] = useState(false)

  // Pulse ring while Active. The ADR specifies a 3-second pulse on the
  // leading edge of new-HITL arrival; we render `animate-ping` for the
  // duration of the Active state, which is a small simplification that
  // keeps the pending count visible at a glance (continuous low-
  // opacity ring) without the cascading-render dance a one-shot 3s
  // timer would require. Once the HITL stream is wired (see
  // `use-hitl-count.ts`), the rule can tighten to one-shot if user
  // testing shows the continuous pulse is too noisy.

  // Hidden state on scroll-down. Read pageYOffset because the scroll
  // event fires on the main element (per AppLayout), not the window
  // — but pageYOffset moves the same way and the velocity heuristic
  // doesn't care which container drove it.
  useEffect(() => {
    if (typeof window === "undefined") return
    let lastY = window.scrollY
    let lastT = performance.now()
    // Listen on capture phase for both window scroll and the AppLayout
    // <main>'s scroll. Capture catches the main scroll bubbling up.
    function onScroll() {
      const now = performance.now()
      const dy = window.scrollY - lastY
      const dt = (now - lastT) / 1000
      if (dt > 0) {
        const v = dy / dt
        if (v > HIDE_VELOCITY) setHidden(true)
        else if (v < -HIDE_VELOCITY / 2) setHidden(false)
      }
      lastY = window.scrollY
      lastT = now
    }
    // Also catch <main> scroll on the app shell since that's where
    // overflow-auto lives.
    const main = document.querySelector("main")
    let mainLastY = main?.scrollTop ?? 0
    function onMainScroll() {
      if (!main) return
      const now = performance.now()
      const dy = main.scrollTop - mainLastY
      const dt = (now - lastT) / 1000
      if (dt > 0) {
        const v = dy / dt
        if (v > HIDE_VELOCITY) setHidden(true)
        else if (v < -HIDE_VELOCITY / 2) setHidden(false)
      }
      mainLastY = main.scrollTop
      lastT = now
    }
    window.addEventListener("scroll", onScroll, { passive: true })
    main?.addEventListener("scroll", onMainScroll, { passive: true })
    return () => {
      window.removeEventListener("scroll", onScroll)
      main?.removeEventListener("scroll", onMainScroll)
    }
  }, [])

  // Virtual keyboard up → hide. visualViewport.height shrinks when the
  // keyboard opens on iOS/Android. Picking a 0.75 ratio as "keyboard
  // open" matches the convention.
  useEffect(() => {
    if (typeof window === "undefined" || !("visualViewport" in window)) return
    const vv = window.visualViewport!
    const baseHeight = vv.height
    function onResize() {
      const ratio = vv.height / baseHeight
      if (ratio < 0.75) setHidden(true)
      else setHidden(false)
    }
    vv.addEventListener("resize", onResize)
    return () => vv.removeEventListener("resize", onResize)
  }, [])

  const handleOpen = useCallback(() => {
    // Increment use count, surface the hint on the 3rd use.
    const next = readUseCount() + 1
    writeUseCount(next)
    if (next === HINT_USE_THRESHOLD && !hintAlreadySeen()) {
      setHintVisible(true)
      markHintSeen()
      window.setTimeout(() => setHintVisible(false), HINT_DURATION_MS)
    }
    // Default tab per ADR: Queue if pending, else Thread. The
    // container's default-tab effect re-applies the rule on scope
    // change, but we set it explicitly here so the open animation
    // shows the right tab from frame 1.
    open({ tab: pendingCount > 0 ? "queue" : "thread" })
  }, [open, pendingCount])

  // Long-press → Inbox. Pointer-down starts a timer; pointer-up cancels
  // it. On long-press fire: vibrate + open at workspace scope on the
  // Queue tab.
  const longPressTimer = useRef<number | null>(null)
  const longPressFiredRef = useRef(false)

  const clearLongPressTimer = useCallback(() => {
    if (longPressTimer.current !== null) {
      window.clearTimeout(longPressTimer.current)
      longPressTimer.current = null
    }
  }, [])

  const onPointerDown = useCallback(() => {
    longPressFiredRef.current = false
    clearLongPressTimer()
    longPressTimer.current = window.setTimeout(() => {
      longPressFiredRef.current = true
      if (typeof navigator !== "undefined" && "vibrate" in navigator) {
        try {
          navigator.vibrate(LONG_PRESS_HAPTIC_MS)
        } catch {
          /* some browsers reject vibrate outside user gesture */
        }
      }
      open({ scope: WORKSPACE_SCOPE, tab: "queue" })
    }, LONG_PRESS_MS)
  }, [open, clearLongPressTimer])

  const onPointerUp = useCallback(() => {
    clearLongPressTimer()
  }, [clearLongPressTimer])

  const onPointerCancel = useCallback(() => {
    clearLongPressTimer()
  }, [clearLongPressTimer])

  const onClick = useCallback(() => {
    // Long-press already opened the chat; suppress the click.
    if (longPressFiredRef.current) {
      longPressFiredRef.current = false
      return
    }
    handleOpen()
  }, [handleOpen])

  const isActive = pendingCount > 0
  const sizeClass = isActive ? "h-[52px] w-[52px]" : "h-10 w-10"
  const visualClass = isActive
    ? "bg-primary text-primary-foreground shadow-lg"
    : "bg-background text-foreground/70 border border-border/60 opacity-75"
  const hiddenClass =
    hidden && !forceVisible
      ? "translate-y-2 opacity-0 pointer-events-none"
      : "translate-y-0 opacity-100"

  // The hint outlives the FAB itself — when the chat opens (FAB
  // unmounts), the hint stays visible above the chat until its timer
  // fires. Both live in the same fixed container; the button is what
  // we hide while the chat is open, not the entire surface.
  return (
    <div
      data-slot="chat-fab-container"
      // Mobile-only positioning. The fab sits above safe-area-inset and
      // 16px in from the right edge per material guidance. md:hidden
      // keeps it off desktop — desktop opens chat via bell + ⌘J.
      className="fixed bottom-[calc(env(safe-area-inset-bottom)+1rem)] right-4 z-40 flex flex-col items-end gap-2 md:hidden"
    >
      {hintVisible && (
        <div
          data-slot="chat-fab-hint"
          role="status"
          className="max-w-[14rem] rounded-md border bg-popover px-3 py-2 text-xs text-popover-foreground shadow-md"
        >
          Hold to jump to the workspace Inbox.
        </div>
      )}
      {!isOpen && (
        <button
          type="button"
          data-slot="chat-fab"
          data-state={isActive ? "active" : "muted"}
          aria-label={
            isActive ? `Open chat — ${pendingCount} pending` : "Open chat"
          }
          onPointerDown={onPointerDown}
          onPointerUp={onPointerUp}
          onPointerCancel={onPointerCancel}
          onPointerLeave={onPointerCancel}
          onClick={onClick}
          className={cn(
            "relative flex items-center justify-center rounded-full transition-all duration-200 will-change-transform",
            sizeClass,
            visualClass,
            hiddenClass,
          )}
        >
          <MessageSquare className={isActive ? "h-6 w-6" : "h-5 w-5"} />
          {isActive && (
            <span
              data-slot="chat-fab-badge"
              className="absolute -right-1 -top-1 flex h-5 min-w-[1.25rem] items-center justify-center rounded-full bg-destructive px-1 text-[10px] font-semibold text-destructive-foreground"
            >
              {pendingCount > 99 ? "99+" : pendingCount}
            </span>
          )}
          {isActive && (
            <span
              data-slot="chat-fab-pulse"
              aria-hidden="true"
              className="absolute inset-0 -m-1 animate-ping rounded-full border-2 border-primary/40"
            />
          )}
        </button>
      )}
    </div>
  )
}
