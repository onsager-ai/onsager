import { useCallback, useEffect, useState } from "react"

function readHashValue(allowed: readonly string[]): string | null {
  if (typeof window === "undefined") return null
  const raw = window.location.hash.replace(/^#/, "")
  return allowed.includes(raw) ? raw : null
}

/**
 * Hash-based tab state. `#tab-name` in the URL selects the active tab so
 * tabs survive reload and deep-link. Falls back to `defaultValue` when
 * the hash is missing or not one of `tabs`.
 *
 * The setter updates `window.location.hash` via `history.replaceState` so
 * tab switches don't pollute the back-button stack.
 */
export function useHashTab<T extends string>(
  tabs: readonly T[],
  defaultValue: T,
): [T, (next: T) => void] {
  const [value, setValue] = useState<T>(() => {
    const fromHash = readHashValue(tabs as readonly string[])
    return (fromHash as T) ?? defaultValue
  })

  useEffect(() => {
    const onHashChange = () => {
      const fromHash = readHashValue(tabs as readonly string[])
      if (fromHash) setValue(fromHash as T)
    }
    window.addEventListener("hashchange", onHashChange)
    return () => window.removeEventListener("hashchange", onHashChange)
  }, [tabs])

  const update = useCallback(
    (next: T) => {
      setValue(next)
      if (typeof window !== "undefined") {
        const url = `${window.location.pathname}${window.location.search}#${next}`
        window.history.replaceState(null, "", url)
      }
    },
    [],
  )

  return [value, update]
}
