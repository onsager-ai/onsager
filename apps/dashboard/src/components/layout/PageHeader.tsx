import {
  createContext,
  useContext,
  useEffect,
  useMemo,
  useState,
  type ReactNode,
} from "react"

// Mobile-only header slot pattern. Pages declare their header content
// once via `usePageHeader({ title, backTo, actions })`; AppLayout's
// mobile bar reads from this context and renders:
//
//   [← backTo OR ☰ sidebar] [title] [...actions]
//
// Desktop is unchanged — pages still render their own page-level
// title + actions block (typically gated `hidden md:flex`). Keeping the
// API mobile-leaning avoids two source-of-truths: the header context is
// always set, but only the mobile chrome consumes it.
export interface PageHeaderState {
  title?: ReactNode
  // Back-arrow target. When set, the mobile header replaces the
  // sidebar trigger with a back button. Pass an absolute path so deep
  // links work — relative `navigate(-1)` is unreliable when the page
  // was opened directly.
  backTo?: string
  // Icon-only buttons rendered at the right of the mobile header.
  // Keep to ≤ 2 items; overflow into a `…` menu if a page needs more.
  actions?: ReactNode
}

interface PageHeaderContextValue extends PageHeaderState {
  setHeader: (state: PageHeaderState) => void
  clearHeader: () => void
}

const PageHeaderContext = createContext<PageHeaderContextValue | null>(null)

export function PageHeaderProvider({ children }: { children: ReactNode }) {
  const [state, setState] = useState<PageHeaderState>({})

  const value = useMemo<PageHeaderContextValue>(
    () => ({
      ...state,
      setHeader: setState,
      clearHeader: () => setState({}),
    }),
    [state],
  )

  return (
    <PageHeaderContext.Provider value={value}>
      {children}
    </PageHeaderContext.Provider>
  )
}

export function usePageHeaderState(): PageHeaderState {
  const ctx = useContext(PageHeaderContext)
  if (!ctx) throw new Error("usePageHeaderState must be used within PageHeaderProvider")
  return { title: ctx.title, backTo: ctx.backTo, actions: ctx.actions }
}

// Pages call this in their render to register their header content.
// State is keyed on the JSON-serializable bits; React-node `title` and
// `actions` are deps too so updates flow through. Cleared on unmount so
// navigating away resets the bar.
export function usePageHeader(state: PageHeaderState) {
  const ctx = useContext(PageHeaderContext)
  if (!ctx) throw new Error("usePageHeader must be used within PageHeaderProvider")
  const { setHeader, clearHeader } = ctx
  const { title, backTo, actions } = state
  useEffect(() => {
    setHeader({ title, backTo, actions })
    return clearHeader
  }, [title, backTo, actions, setHeader, clearHeader])
}
