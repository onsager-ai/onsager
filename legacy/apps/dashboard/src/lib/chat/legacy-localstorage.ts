// One-release migration helper (ADR 0020 § Storage schema change).
//
// Pre-#470 chat history lived in localStorage keyed by
// `onsager.chat.<user_id>.<workspace_id>` (see `chat-storage.ts`).
// The new server-side store is keyed by
// `(user_id, scope_type, scope_id)`. We do not move the data — the
// ADR commits to a fresh-start migration. Instead, the workspace-
// scoped Thread surfaces a "previous conversations" link that, when
// clicked, renders the legacy turns inline for the user to copy out.
//
// This module is the read side. Writes to the legacy key were removed
// when the page mounts were retired (#457); existing keys remain
// inert in users' browsers until they clear them. The link disappears
// once the user dismisses it or once there are no legacy keys for
// the current user.

import { type StoredTurn } from "./chat-storage"

const KEY_PREFIX = "onsager.chat."
const DISMISS_KEY = "onsager.chat.legacy_dismissed"

export interface LegacyConversation {
  storageKey: string
  workspaceIdHint: string
  turns: StoredTurn[]
}

/**
 * Return every legacy chat conversation in localStorage for the
 * named user. The second key segment is the workspace id (or, in
 * FTUE entries, `draft:<id>` or `draft:empty`); we expose it as a
 * hint so the UI can group/label by workspace without re-loading.
 */
export function readLegacyConversations(
  userId: string | null,
): LegacyConversation[] {
  if (!userId || typeof window === "undefined") return []
  const prefix = `${KEY_PREFIX}${userId}.`
  const out: LegacyConversation[] = []
  try {
    for (let i = 0; i < window.localStorage.length; i++) {
      const k = window.localStorage.key(i)
      if (!k || !k.startsWith(prefix)) continue
      const raw = window.localStorage.getItem(k)
      if (!raw) continue
      let parsed: unknown
      try {
        parsed = JSON.parse(raw)
      } catch {
        continue
      }
      if (!Array.isArray(parsed) || parsed.length === 0) continue
      out.push({
        storageKey: k,
        workspaceIdHint: k.slice(prefix.length),
        turns: parsed as StoredTurn[],
      })
    }
  } catch {
    // localStorage can throw in private mode — return what we have
  }
  return out
}

export function hasLegacyConversations(userId: string | null): boolean {
  return readLegacyConversations(userId).length > 0
}

export function isLegacyLinkDismissed(): boolean {
  if (typeof window === "undefined") return false
  try {
    return window.localStorage.getItem(DISMISS_KEY) === "1"
  } catch {
    return false
  }
}

export function dismissLegacyLink(): void {
  if (typeof window === "undefined") return
  try {
    window.localStorage.setItem(DISMISS_KEY, "1")
  } catch {
    // best-effort
  }
}
