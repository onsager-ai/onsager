// Serializable turn types for localStorage persistence. The runtime
// ChatTurn / ToolCallEntry types in ChatPage.tsx include non-serializable
// fields (McpToolBinding function refs, reconstructed HitlCard objects).
// This layer stores only what is needed to reconstruct full state on load.

import type { HitlCardState } from "@/components/chat/hitl-types"

export interface StoredToolCall {
  id: string
  toolName: string
  input: Record<string, unknown>
  state: HitlCardState
  resultText?: string
  errorMessage?: string
}

export interface StoredTurn {
  id: string
  userContent: string
  assistantContent?: string
  toolCalls: StoredToolCall[]
  error?: string
}

export function chatStorageKey(userId: string, workspaceId: string): string {
  return `onsager.chat.${userId}.${workspaceId}`
}

export function loadStoredTurns(key: string): StoredTurn[] {
  if (typeof window === "undefined") return []
  try {
    const raw = window.localStorage.getItem(key)
    if (!raw) return []
    const parsed: unknown = JSON.parse(raw)
    return Array.isArray(parsed) ? (parsed as StoredTurn[]) : []
  } catch {
    return []
  }
}

export function saveStoredTurns(key: string, turns: StoredTurn[]): void {
  if (typeof window === "undefined") return
  try {
    window.localStorage.setItem(key, JSON.stringify(turns))
  } catch {
    // quota exhaustion or private mode — silently fail
  }
}
