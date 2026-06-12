// Onsager service worker — HITL push notifications (ADR 0020 § Invocation
// channels, spec #472).
//
// Lives at the app origin so the same service worker can claim every
// dashboard tab. Two responsibilities:
//
//   1. `push` event handler. Decodes the JSON payload portal dispatches
//      (see crates/onsager-portal/src/push.rs `PushPayload`), shows a
//      browser notification with a single `Open` action.
//   2. `notificationclick` handler. Translates the click into a focus-
//      or-open of the dashboard at the carried `click_path` plus the
//      `onsager_chat=open&onsager_chat_scope=...&onsager_hitl_id=...`
//      query params the ChatDeepLinkHandler consumes.
//
// Open-only action shape per ADR 0020 § Notification action buttons —
// Approve / Reject are deferred until the action-button-binding ADR
// lands.

const TAG_PREFIX = "onsager-hitl-"

self.addEventListener("install", (event) => {
  // Activate as soon as the previous worker releases — pre-launch we
  // never wait for slow clients to drain.
  event.waitUntil(self.skipWaiting())
})

self.addEventListener("activate", (event) => {
  event.waitUntil(self.clients.claim())
})

self.addEventListener("push", (event) => {
  if (!event.data) return
  let payload
  try {
    payload = event.data.json()
  } catch {
    // Fall back to a generic notification — keeps the surface honest
    // if a malformed push arrives in production.
    payload = {
      title: "Onsager",
      body: event.data.text() || "You have a new notification.",
      scope_key: "workspace",
      click_path: "/",
    }
  }

  const title = payload.title || "Onsager"
  const tag = payload.hitl_id ? `${TAG_PREFIX}${payload.hitl_id}` : undefined
  const options = {
    body: payload.body || "",
    tag,
    // `renotify: true` re-rings + re-pops even when the tag matches —
    // operators want to know about new HITLs even if the same tag is
    // already on the lock screen.
    renotify: !!tag,
    icon: "/favicon.svg",
    badge: "/favicon.svg",
    data: {
      hitl_id: payload.hitl_id,
      scope_key: payload.scope_key,
      click_path: payload.click_path,
      kind: payload.kind,
    },
    actions: [
      // Open-only fallback (phase 0). When the action-button-binding ADR
      // lands, approval-kind HITLs add `approve` + `reject` actions; the
      // ChatDeepLinkHandler stays unchanged because Open keeps the deep
      // link as the source of truth.
      { action: "open", title: "Open" },
    ],
  }

  event.waitUntil(self.registration.showNotification(title, options))
})

self.addEventListener("notificationclick", (event) => {
  event.notification.close()
  const data = event.notification.data || {}
  const params = new URLSearchParams()
  params.set("onsager_chat", "open")
  if (data.scope_key) params.set("onsager_chat_scope", data.scope_key)
  if (data.hitl_id) params.set("onsager_hitl_id", data.hitl_id)
  const path = data.click_path || "/"
  const sep = path.includes("?") ? "&" : "?"
  const url = new URL(`${path}${sep}${params.toString()}`, self.location.origin).toString()

  event.waitUntil(
    (async () => {
      const all = await self.clients.matchAll({
        type: "window",
        includeUncontrolled: true,
      })
      // Prefer focusing an existing window that's already on the app
      // origin; navigate it to the deep-link URL so the chat opens there.
      for (const client of all) {
        if (new URL(client.url).origin === self.location.origin) {
          await client.focus()
          if ("navigate" in client) {
            try {
              await client.navigate(url)
            } catch {
              // navigate() rejects when crossing origin or when the
              // client is uncontrolled. Fall through to openWindow.
            }
          }
          return
        }
      }
      await self.clients.openWindow(url)
    })(),
  )
})
