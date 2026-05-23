import { request, scoped } from "./client"
import type { PushConfigResponse } from "./generated/PushConfigResponse"
import type { PushSubscribeResponse } from "./generated/PushSubscribeResponse"

// Push Notification API client. Mirrors the portal endpoints in
// `crates/onsager-portal/src/handlers/push.rs` (ADR 0020 / spec #472).

export interface PushSubscribeRequest {
  workspaceId: string
  endpoint: string
  p256dh: string
  auth: string
  userAgent?: string
}

export const push = {
  getConfig: (): Promise<PushConfigResponse> =>
    request<PushConfigResponse>("/push/config"),

  subscribe: (body: PushSubscribeRequest): Promise<PushSubscribeResponse> =>
    request<PushSubscribeResponse>("/push/subscribe", {
      method: "POST",
      body: JSON.stringify({
        workspace_id: body.workspaceId,
        endpoint: body.endpoint,
        p256dh: body.p256dh,
        auth: body.auth,
        user_agent: body.userAgent,
      }),
    }),

  unsubscribe: (endpoint: string): Promise<void> =>
    request<void>("/push/subscribe", {
      method: "DELETE",
      body: JSON.stringify({ endpoint }),
    }),

  /** Dev convenience — gated to dev-login deploys server-side. */
  testDispatch: (
    workspaceId: string,
    clickPath: string,
    scopeKey?: string,
  ): Promise<{ delivered: number }> =>
    request<{ delivered: number }>(`/push/test${scoped(workspaceId)}`, {
      method: "POST",
      body: JSON.stringify({
        workspace_id: workspaceId,
        click_path: clickPath,
        scope_key: scopeKey,
      }),
    }),
}
