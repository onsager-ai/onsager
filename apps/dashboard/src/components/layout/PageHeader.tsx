import {
  createContext,
  useCallback,
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
// The contexts are deliberately split: pages subscribe only to the
// stable setter (PageHeaderSetContext, value never changes), so writes
// from the hook don't re-render the page itself. Only the bar (which
// reads PageHeaderValueContext) re-renders. Without this split, every
// time a page passes a fresh JSX `actions` node, setState would
// re-render the page, which would create new JSX, which would fire the
// effect again — an infinite loop.
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

interface PageHeaderSetters {
  setHeader: (state: PageHeaderState) => void
  clearHeader: () => void
}

const PageHeaderValueContext = createContext<PageHeaderState>({})
const PageHeaderSetContext = createContext<PageHeaderSetters | null>(null)

export function PageHeaderProvider({ children }: { children: ReactNode }) {
  const [state, setState] = useState<PageHeaderState>({})

  const setters = useMemo<PageHeaderSetters>(
    () => ({
      setHeader: (next) =>
        setState((prev) =>
          prev.title === next.title &&
          prev.backTo === next.backTo &&
          prev.actions === next.actions
            ? prev
            : next,
        ),
      clearHeader: () => setState({}),
    }),
    [],
  )

  return (
    <PageHeaderSetContext.Provider value={setters}>
      <PageHeaderValueContext.Provider value={state}>
        {children}
      </PageHeaderValueContext.Provider>
    </PageHeaderSetContext.Provider>
  )
}

// eslint-disable-next-line react-refresh/only-export-components
export function usePageHeaderState(): PageHeaderState {
  return useContext(PageHeaderValueContext)
}

// Pages call this in their render to register their header content.
// Cleared on unmount so navigating away resets the bar. Note: pages
// passing JSX `actions` should `useMemo` the node (or this hook will
// re-set on every render — wasteful but not a loop, since pages don't
// subscribe to the value context).
//
// Safe outside a provider — silently no-ops, so smoke tests can mount
// pages standalone without wrapping every test in AppLayout.
// eslint-disable-next-line react-refresh/only-export-components
export function usePageHeader(state: PageHeaderState) {
  const setters = useContext(PageHeaderSetContext)
  const { title, backTo, actions } = state

  const setHeaderCb = useCallback(
    () => setters?.setHeader({ title, backTo, actions }),
    [setters, title, backTo, actions],
  )

  useEffect(() => {
    if (!setters) return
    setHeaderCb()
    return setters.clearHeader
  }, [setters, setHeaderCb])
}
