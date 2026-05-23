import { request, scoped } from "./client"
import type { ChatThreadResponse } from "./generated/ChatThreadResponse"
import type { ChatSearchResponse } from "./generated/ChatSearchResponse"
import type { ChatMessage } from "./generated/ChatMessage"

// Scope shape consumed by `GET /api/chat/thread`. Mirrors the portal
// `ThreadQuery` (`crates/onsager-portal/src/handlers/chat.rs`): the
// `workspace` scope omits `scope_id`; every other scope requires one.
// Kept as a plain object (no class) so the value can flow through React
// state and React Query keys directly.
export type ChatScopeWire =
  | { scope_type: "workspace" }
  | { scope_type: "workflow"; scope_id: string }
  | { scope_type: "run"; scope_id: string }
  | { scope_type: "verdict"; scope_id: string }

export type ChatMessageRole = "user" | "assistant" | "tool"

export const chat = {
  getThread: (
    workspaceId: string,
    scope: ChatScopeWire,
  ): Promise<ChatThreadResponse> => {
    const extra: Record<string, string> = { scope_type: scope.scope_type }
    if (scope.scope_type !== "workspace") {
      extra.scope_id = scope.scope_id
    }
    return request<ChatThreadResponse>(`/chat/thread${scoped(workspaceId, extra)}`)
  },

  appendMessage: (
    threadId: string,
    workspaceId: string,
    role: ChatMessageRole,
    content: string,
    toolCallId?: string,
  ): Promise<ChatMessage> =>
    request<ChatMessage>(`/chat/thread/${encodeURIComponent(threadId)}/messages`, {
      method: "POST",
      body: JSON.stringify({
        workspace_id: workspaceId,
        role,
        content,
        tool_call_id: toolCallId,
      }),
    }),

  search: (
    workspaceId: string,
    q: string,
    limit?: number,
  ): Promise<ChatSearchResponse> =>
    request<ChatSearchResponse>(
      `/chat/search${scoped(workspaceId, { q, limit })}`,
    ),
}
