import { useCallback, useEffect, useRef, useState } from "react"

// Progressive in-page reveal for long anchored sections on
// `WorkflowDetailPage` (#466). Holds a `visibleCount` that starts at
// `initial`, grows by `step` whenever the returned sentinel ref enters
// the viewport, and snaps to `total` when `revealAll` is called.
//
// Returning the sentinel ref instead of accepting one lets the caller
// drop it next to a "Show all" button without threading observer wiring
// through props.
export function useLazyReveal({
  initial,
  step,
  total,
}: {
  initial: number
  step: number
  total: number
}) {
  const [rawVisibleCount, setVisibleCount] = useState(initial)
  // Clamp at render time so the invariant `visibleCount <= total`
  // holds even when consumers pass a shrinking `total` (e.g. filter
  // change). When `total = 0` this correctly yields `visibleCount = 0`
  // — empty list, no Show-all button.
  const visibleCount = Math.min(rawVisibleCount, total)
  const sentinelRef = useRef<HTMLDivElement | null>(null)

  useEffect(() => {
    const node = sentinelRef.current
    if (!node) return
    if (visibleCount >= total) return
    if (typeof IntersectionObserver === "undefined") return

    const observer = new IntersectionObserver(
      (entries) => {
        for (const entry of entries) {
          if (entry.isIntersecting) {
            setVisibleCount((c) => Math.min(c + step, total))
          }
        }
      },
      { rootMargin: "200px 0px" },
    )
    observer.observe(node)
    return () => observer.disconnect()
  }, [visibleCount, step, total])

  const revealAll = useCallback(() => {
    setVisibleCount(total)
  }, [total])

  return {
    visibleCount,
    sentinelRef,
    hasMore: visibleCount < total,
    revealAll,
  }
}
