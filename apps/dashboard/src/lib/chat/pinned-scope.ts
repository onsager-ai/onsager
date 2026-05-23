// Persistence layer for the pin affordance (ADR 0020 § Contextual
// auto-scope). When the user pins, the active scope is locked against
// route navigation until the user unpins or switches workspaces. We
// persist the pin per-user, per-workspace so a phone-then-laptop hop
// inside the same workspace keeps the same lock.
//
// Server-side persistence is the right long-term home (would survive
// `localStorage` clears and cross-device), but the storage backend
// from spec #470 covers messages only — there is no user-prefs row
// to attach to. localStorage is fine here: pin state is a UI lock,
// not authoritative state. If the user clears storage, the chat
// simply resumes the auto-scope rule and they re-pin.

import { type ChatScope, parseScopeKey, scopeKey } from "./scope"

function key(userId: string, workspaceId: string): string {
  return `onsager.chat.pinned_scope.${userId}.${workspaceId}`
}

export function readPinnedScope(
  userId: string | null,
  workspaceId: string | null,
): ChatScope | null {
  if (!userId || !workspaceId || typeof window === "undefined") return null
  try {
    const raw = window.localStorage.getItem(key(userId, workspaceId))
    return raw ? parseScopeKey(raw) : null
  } catch {
    return null
  }
}

export function writePinnedScope(
  userId: string,
  workspaceId: string,
  scope: ChatScope,
): void {
  if (typeof window === "undefined") return
  try {
    window.localStorage.setItem(key(userId, workspaceId), scopeKey(scope))
  } catch {
    // private mode / quota — pin reverts to auto-scope, acceptable
  }
}

export function clearPinnedScope(userId: string, workspaceId: string): void {
  if (typeof window === "undefined") return
  try {
    window.localStorage.removeItem(key(userId, workspaceId))
  } catch {
    // best-effort
  }
}
