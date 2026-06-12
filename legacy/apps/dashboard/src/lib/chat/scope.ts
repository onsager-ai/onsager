// ChatScope — what view the chat is currently bound to. The four scope
// types come straight from ADR 0020's contextual-auto-scope table.
//
// `workspace` is the rolling all-of-workspace thread (also the Inbox
// destination). The other three scopes carry a non-empty `id`; the
// portal enforces this server-side at the `chat_threads` unique key.

export type ChatScopeType = "workspace" | "workflow" | "run" | "verdict"

export interface ChatScope {
  type: ChatScopeType
  // `null` for `workspace`; non-empty string for everything else.
  id: string | null
}

export const WORKSPACE_SCOPE: ChatScope = { type: "workspace", id: null }

export function scopesEqual(a: ChatScope, b: ChatScope): boolean {
  return a.type === b.type && a.id === b.id
}

/** Stable string form of a scope, for React Query keys + localStorage. */
export function scopeKey(scope: ChatScope): string {
  return scope.id ? `${scope.type}:${scope.id}` : scope.type
}

export function parseScopeKey(s: string): ChatScope | null {
  if (s === "workspace") return WORKSPACE_SCOPE
  const idx = s.indexOf(":")
  if (idx === -1) return null
  const type = s.slice(0, idx)
  const id = s.slice(idx + 1)
  if ((type === "workflow" || type === "run" || type === "verdict") && id) {
    return { type, id }
  }
  return null
}
