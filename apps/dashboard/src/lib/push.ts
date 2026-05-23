import { api } from "./api"

// Browser-side Web Push registration helpers (ADR 0020 § Invocation
// channels, spec #472).
//
// The dashboard ships a service worker at `/sw.js`. Once the user opts
// in via the settings toggle, we register the worker, ask the browser
// for a `PushSubscription`, and POST its JSON shape to portal so the
// dispatch helper can encrypt the next payload for this endpoint.

const SW_PATH = "/sw.js"

export type PushPermission = "default" | "granted" | "denied"

export interface PushSupport {
  /** Both ServiceWorker + PushManager + Notification APIs are available. */
  supported: boolean
  /** Current permission state for `Notification`. */
  permission: PushPermission
  /** The backend has VAPID keys configured. Pulled from `/api/push/config`. */
  enabled: boolean
  vapidPublicKey: string | null
}

export async function describePushSupport(): Promise<PushSupport> {
  const supported =
    typeof window !== "undefined" &&
    "serviceWorker" in navigator &&
    "PushManager" in window &&
    "Notification" in window
  let permission: PushPermission = "default"
  if (supported) {
    permission = Notification.permission as PushPermission
  }
  let enabled = false
  let vapidPublicKey: string | null = null
  try {
    const config = await api.push.getConfig()
    enabled = config.enabled
    vapidPublicKey = config.vapid_public_key
  } catch {
    // Treat config failures as "disabled" so the UI stays hidden.
  }
  return { supported, permission, enabled, vapidPublicKey }
}

/**
 * Register the service worker. Idempotent — calling twice returns the
 * existing registration. The worker activates immediately because the
 * SW itself calls `skipWaiting()` + `clients.claim()`.
 */
export async function ensureServiceWorker(): Promise<ServiceWorkerRegistration> {
  if (!("serviceWorker" in navigator)) {
    throw new Error("Service workers are not supported in this browser")
  }
  const existing = await navigator.serviceWorker.getRegistration(SW_PATH)
  if (existing) return existing
  return navigator.serviceWorker.register(SW_PATH, { scope: "/" })
}

/**
 * Ask for notification permission. Resolves with the resulting state —
 * `denied` is a terminal state on most browsers (the user has to flip
 * it manually in browser settings).
 */
export async function requestPermission(): Promise<PushPermission> {
  if (!("Notification" in window)) return "denied"
  const result = await Notification.requestPermission()
  return result as PushPermission
}

/**
 * Subscribe the current browser to Web Push and POST the result to
 * portal. Returns the persisted subscription id on success.
 */
export async function subscribePush(
  workspaceId: string,
  vapidPublicKey: string,
): Promise<string> {
  const registration = await ensureServiceWorker()
  // Reuse any existing subscription — re-subscribing the same browser
  // would just upsert the same row, but skipping the trip keeps the
  // network quiet.
  const existing = await registration.pushManager.getSubscription()
  const sub =
    existing ??
    (await registration.pushManager.subscribe({
      userVisibleOnly: true,
      applicationServerKey: urlBase64ToUint8Array(vapidPublicKey),
    }))
  const json = sub.toJSON()
  if (!json.endpoint || !json.keys?.p256dh || !json.keys?.auth) {
    throw new Error("Browser returned an incomplete push subscription")
  }
  const res = await api.push.subscribe({
    workspaceId,
    endpoint: json.endpoint,
    p256dh: json.keys.p256dh,
    auth: json.keys.auth,
    userAgent: navigator.userAgent,
  })
  return res.id
}

/**
 * Unsubscribe the current browser. Best-effort — the portal row is
 * dropped even when the browser-side unsubscribe fails (e.g. user
 * cleared site data).
 */
export async function unsubscribePush(): Promise<void> {
  if (!("serviceWorker" in navigator)) return
  const registration = await navigator.serviceWorker.getRegistration(SW_PATH)
  if (!registration) return
  const sub = await registration.pushManager.getSubscription()
  if (!sub) return
  try {
    await api.push.unsubscribe(sub.endpoint)
  } finally {
    await sub.unsubscribe()
  }
}

/** Read whether the browser currently holds an active subscription. */
export async function isCurrentlySubscribed(): Promise<boolean> {
  if (!("serviceWorker" in navigator)) return false
  const reg = await navigator.serviceWorker.getRegistration(SW_PATH)
  const sub = await reg?.pushManager.getSubscription()
  return !!sub
}

// VAPID public keys come from portal as a base64-url string; the
// browser's `applicationServerKey` wants an `ArrayBuffer`-backed view.
// Allocating a fresh ArrayBuffer (not the default Uint8Array buffer type)
// keeps TS's stricter `ArrayBufferView<ArrayBuffer>` typing happy.
function urlBase64ToUint8Array(base64: string): Uint8Array<ArrayBuffer> {
  const padding = "=".repeat((4 - (base64.length % 4)) % 4)
  const base64Std = (base64 + padding).replace(/-/g, "+").replace(/_/g, "/")
  const raw = atob(base64Std)
  const buf = new ArrayBuffer(raw.length)
  const out = new Uint8Array(buf)
  for (let i = 0; i < raw.length; i++) out[i] = raw.charCodeAt(i)
  return out
}
