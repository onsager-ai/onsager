import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useState,
  type ReactNode,
} from "react"
import type { ChatScope } from "./scope"

// ChatUi — open/close state for the global ChatContainer.
//
// Invocation channels (#472 FAB / bell / Ask + push deep-link, #473 ⌘K)
// all call `open()` to bring up the mount; the close button + ⌘J close
// it. Kept in a tiny context so any tree node can trigger the chat
// without prop-drilling, and so the keyboard handler can live at the
// provider root.
//
// `scopeOverride` and `highlightHitlId` are one-shot — they apply to
// the current open() call, and clear on close. The auto-scope hook
// honors the override when set (per ADR 0020: invocation channels can
// pin a specific scope when the user explicitly enters at that scope).

export type ChatTab = "queue" | "thread"

export interface ChatOpenOptions {
  /** Force this scope until close. Auto-scope resumes after close. */
  scope?: ChatScope
  tab?: ChatTab
  /** When set, the Queue tab scrolls to / highlights this HITL card. */
  highlightHitlId?: string
  /** One-shot pre-fill for the Thread composer (⌘K → `ask: …`, #473). */
  prefill?: string
}

export interface ChatUi {
  isOpen: boolean
  tab: ChatTab
  /** Set by `open({ scope })`; cleared on close. Null when not overridden. */
  scopeOverride: ChatScope | null
  /** Set by `open({ highlightHitlId })`; cleared on close. */
  highlightHitlId: string | null
  /** Set by `open({ prefill })`; cleared on close. */
  prefill: string | null
  open(options?: ChatOpenOptions): void
  close(): void
  toggle(): void
  setTab(tab: ChatTab): void
}

const ChatUiContext = createContext<ChatUi | null>(null)

export function ChatUiProvider({ children }: { children: ReactNode }) {
  const [isOpen, setIsOpen] = useState(false)
  const [tab, setTab] = useState<ChatTab>("thread")
  const [scopeOverride, setScopeOverride] = useState<ChatScope | null>(null)
  const [highlightHitlId, setHighlightHitlId] = useState<string | null>(null)
  const [prefill, setPrefill] = useState<string | null>(null)

  const open = useCallback((options?: ChatOpenOptions) => {
    if (options?.tab) setTab(options.tab)
    setScopeOverride(options?.scope ?? null)
    setHighlightHitlId(options?.highlightHitlId ?? null)
    setPrefill(options?.prefill ?? null)
    setIsOpen(true)
  }, [])
  const close = useCallback(() => {
    setIsOpen(false)
    setScopeOverride(null)
    setHighlightHitlId(null)
    setPrefill(null)
  }, [])
  const toggle = useCallback(() => {
    setIsOpen((v) => {
      if (v) {
        setScopeOverride(null)
        setHighlightHitlId(null)
        setPrefill(null)
      }
      return !v
    })
  }, [])

  // ⌘J / Ctrl+J — toggle the global chat on desktop. The mobile FAB is
  // the primary opener on phones; desktop has both the bell and ⌘J.
  useEffect(() => {
    function onKeyDown(e: KeyboardEvent) {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "j") {
        e.preventDefault()
        toggle()
      }
    }
    window.addEventListener("keydown", onKeyDown)
    return () => window.removeEventListener("keydown", onKeyDown)
  }, [toggle])

  const value = useMemo<ChatUi>(
    () => ({
      isOpen,
      tab,
      scopeOverride,
      highlightHitlId,
      prefill,
      open,
      close,
      toggle,
      setTab,
    }),
    [isOpen, tab, scopeOverride, highlightHitlId, prefill, open, close, toggle],
  )
  return <ChatUiContext.Provider value={value}>{children}</ChatUiContext.Provider>
}

// eslint-disable-next-line react-refresh/only-export-components
export function useChatUi(): ChatUi {
  const ctx = useContext(ChatUiContext)
  if (!ctx) {
    throw new Error("useChatUi must be used inside a ChatUiProvider")
  }
  return ctx
}

/** Optional variant for trees that may render before the provider (tests). */
// eslint-disable-next-line react-refresh/only-export-components
export function useOptionalChatUi(): ChatUi | null {
  return useContext(ChatUiContext)
}
