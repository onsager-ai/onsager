import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useState,
  type ReactNode,
} from "react"

// ChatUi — open/close state for the global ChatContainer.
//
// Invocation channels (#472 FAB / bell / Ask, #473 ⌘K) all call
// `open()` to bring up the mount; the close button + ⌘J close it.
// Kept in a tiny context so any tree node can trigger the chat
// without prop-drilling, and so the keyboard handler can live at
// the provider root.

export type ChatTab = "queue" | "thread"

export interface ChatUi {
  isOpen: boolean
  /** Currently selected tab — invocation channels can request one. */
  tab: ChatTab
  open(options?: { tab?: ChatTab }): void
  close(): void
  toggle(): void
  setTab(tab: ChatTab): void
}

const ChatUiContext = createContext<ChatUi | null>(null)

export function ChatUiProvider({ children }: { children: ReactNode }) {
  const [isOpen, setIsOpen] = useState(false)
  const [tab, setTab] = useState<ChatTab>("thread")

  const open = useCallback((options?: { tab?: ChatTab }) => {
    if (options?.tab) setTab(options.tab)
    setIsOpen(true)
  }, [])
  const close = useCallback(() => setIsOpen(false), [])
  const toggle = useCallback(() => setIsOpen((v) => !v), [])

  // ⌘J / Ctrl+J — toggle the global chat on desktop. The mobile
  // FAB lands in #472; on desktop the keyboard shortcut is the
  // primary opener until the bell ships.
  useEffect(() => {
    function onKeyDown(e: KeyboardEvent) {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "j") {
        e.preventDefault()
        setIsOpen((v) => !v)
      }
    }
    window.addEventListener("keydown", onKeyDown)
    return () => window.removeEventListener("keydown", onKeyDown)
  }, [])

  const value = useMemo<ChatUi>(
    () => ({ isOpen, tab, open, close, toggle, setTab }),
    [isOpen, tab, open, close, toggle],
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
