import { useCallback, useEffect, useState } from "react"

function readHashValue(allowed: readonly string[]): string | null {
  if (typeof window === "undefined") return null
  const raw = window.location.hash.replace(/^#/, "")
  return allowed.includes(raw) ? raw : null
}

/**
 * Hash-based tab state. `#tab-name` in the URL selects the active tab so
 * tabs survive reload and deep-link. Falls back to `defaultValue` when
 * the hash is missing or not one of `tabs` — both on first render and
 * when the user clears or edits the hash to something unrecognized.
 *
 * The returned setter accepts an unknown string and validates it against
 * `tabs`, so callers can wire it directly to e.g. `Tabs.onValueChange`
 * without an unchecked cast. Invalid values are dropped silently.
 *
 * Updates use `history.replaceState` so tab switches don't pollute the
 * back-button stack.
 */
export function useHashTab<T extends string>(
  tabs: readonly T[],
  defaultValue: T,
): [T, (next: string) => void] {
  const [value, setValue] = useState<T>(() => {
    const fromHash = readHashValue(tabs as readonly string[])
    return (fromHash as T | null) ?? defaultValue
  })

  useEffect(() => {
    const onHashChange = () => {
      const fromHash = readHashValue(tabs as readonly string[])
      setValue((fromHash as T | null) ?? defaultValue)
    }
    window.addEventListener("hashchange", onHashChange)
    return () => window.removeEventListener("hashchange", onHashChange)
  }, [tabs, defaultValue])

  const update = useCallback(
    (next: string) => {
      if (!(tabs as readonly string[]).includes(next)) return
      const validated = next as T
      setValue(validated)
      if (typeof window !== "undefined") {
        const url = `${window.location.pathname}${window.location.search}#${validated}`
        window.history.replaceState(null, "", url)
      }
    },
    [tabs],
  )

  return [value, update]
}
