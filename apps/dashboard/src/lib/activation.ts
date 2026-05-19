// FTUE activation instrumentation (spec #404).
//
// Four events — `ftue.inspected`, `ftue.drafted`, `ftue.bound`,
// `ftue.activated` — collapse onto one append-only sink at
// `POST /api/activation`. The dashboard owns the first three; the
// fourth is emitted server-side by the portal spine listener (see
// `crates/onsager-portal/src/listeners/workflow_activated.rs`).
//
// Fire-once policy is enforced server-side via a UNIQUE constraint on
// the dedup_key derived from `(event, user_id || anonymous_id,
// primary-context-id)`. The client-side `firedOnce` cache below is a
// performance optimization (avoid the request when we know we've fired
// already in this browser) — not a correctness mechanism. Logging
// out, clearing localStorage, or another device replaying the same
// rung lands a duplicate POST that the server drops silently.
//
// Privacy:
// - `anonymous_id` is a random UUID generated once per browser. It is
//   not a fingerprint; clearing site storage forgets it.
// - `context` carries identifiers (draft_id, workflow_id) but no PII
//   (no email, no repo content, no chat-turn content) — the spec's
//   Privacy section binds this contract.
// - OSS opt-in: when `build-info.is_oss === true`, `trackActivation`
//   short-circuits to a no-op unless the user has explicitly opted
//   in via `setOssTelemetryOptIn(true)`. Cloud users are auto-opted
//   in by ToS acceptance.

import { api } from "@/lib/api"
import { fetchBuildInfo } from "@/lib/build-info"

const ANON_ID_KEY = "onsager.anon_id"
const FIRED_ONCE_KEY = "onsager.activation.fired"
const OSS_OPT_IN_KEY = "onsager.activation.oss_opt_in"

export type ActivationEvent =
  | "ftue.inspected"
  | "ftue.drafted"
  | "ftue.bound"
  | "ftue.activated"

export type ActivationSurface = "landing" | "chat" | "dialog" | "spine"

export interface ActivationContext {
  draft_id?: string
  workspace_id?: string
  workflow_id?: string
  template_id?: string
  run_id?: string
  terminal_status?: "completed" | "failed" | "cancelled"
}

/** Read the localStorage UUID, generating one on first use. */
export function getAnonymousId(): string {
  if (typeof window === "undefined") return "ssr"
  try {
    let id = window.localStorage.getItem(ANON_ID_KEY)
    if (!id) {
      id =
        typeof crypto !== "undefined" && "randomUUID" in crypto
          ? crypto.randomUUID()
          : `anon_${Math.random().toString(36).slice(2)}_${Date.now()}`
      window.localStorage.setItem(ANON_ID_KEY, id)
    }
    return id
  } catch {
    // Private mode / quota — return an ephemeral id rather than
    // throwing through every call site.
    return "anon-ephemeral"
  }
}

/** OSS telemetry opt-in toggle. Cloud ignores this. */
export function isOssTelemetryOptedIn(): boolean {
  if (typeof window === "undefined") return false
  try {
    return window.localStorage.getItem(OSS_OPT_IN_KEY) === "true"
  } catch {
    return false
  }
}

export function setOssTelemetryOptIn(optedIn: boolean): void {
  if (typeof window === "undefined") return
  try {
    if (optedIn) {
      window.localStorage.setItem(OSS_OPT_IN_KEY, "true")
    } else {
      window.localStorage.removeItem(OSS_OPT_IN_KEY)
    }
  } catch {
    /* ignore */
  }
}

/**
 * Client-side fire-once cache. The set of `(event, primary-id)` keys
 * we've already POSTed in this browser. Stored under one localStorage
 * key as a JSON array. Capped at 1000 entries — well above the four
 * lifetime rungs per (draft|workflow) — to keep blob size bounded if
 * a user creates many drafts.
 */
function loadFiredOnce(): Set<string> {
  if (typeof window === "undefined") return new Set()
  try {
    const raw = window.localStorage.getItem(FIRED_ONCE_KEY)
    if (!raw) return new Set()
    const arr = JSON.parse(raw) as unknown
    return new Set(Array.isArray(arr) ? (arr as string[]) : [])
  } catch {
    return new Set()
  }
}

function saveFiredOnce(set: Set<string>): void {
  if (typeof window === "undefined") return
  try {
    const arr = Array.from(set).slice(-1000)
    window.localStorage.setItem(FIRED_ONCE_KEY, JSON.stringify(arr))
  } catch {
    /* ignore */
  }
}

function dedupKey(event: ActivationEvent, context: ActivationContext): string {
  switch (event) {
    case "ftue.inspected":
      return event
    case "ftue.drafted":
    case "ftue.bound":
      return `${event}|${context.draft_id ?? ""}`
    case "ftue.activated":
      return `${event}|${context.workflow_id ?? ""}`
  }
}

/**
 * Fire one activation event. No-op when:
 * - `event === "ftue.activated"` — emitted server-side only.
 * - This (event, primary-id) already fired in this browser.
 * - OSS without explicit opt-in.
 *
 * Best-effort: a 4xx / 5xx response is logged at debug and swallowed.
 * The funnel may miss the row; the product flow must not break.
 */
export async function trackActivation(
  event: ActivationEvent,
  surface: ActivationSurface,
  context: ActivationContext = {},
): Promise<void> {
  if (event === "ftue.activated") return

  const fired = loadFiredOnce()
  const key = dedupKey(event, context)
  if (fired.has(key)) return

  const buildInfo = await fetchBuildInfo()
  const path: "cloud" | "oss" = buildInfo.is_oss ? "oss" : "cloud"
  if (path === "oss" && !isOssTelemetryOptedIn()) return

  try {
    // `event === "ftue.activated"` is ruled out above; the typed
    // request union accepts the remaining three.
    await api.recordActivation({
      event: event as "ftue.inspected" | "ftue.drafted" | "ftue.bound",
      occurred_at: new Date().toISOString(),
      anonymous_id: getAnonymousId(),
      surface,
      path,
      context: context as Record<string, unknown>,
    })
    fired.add(key)
    saveFiredOnce(fired)
  } catch (err) {
    console.debug("[activation] request failed", err)
  }
}

/**
 * Synchronous client-side check: "has this rung already fired in this
 * browser?". Used by the OSS opt-in prompt UX — the first time the
 * user hits the Drafted threshold, surface the opt-in copy *once*.
 * The actual POST still re-checks the cache before sending.
 */
export function hasFiredLocally(
  event: ActivationEvent,
  context: ActivationContext = {},
): boolean {
  return loadFiredOnce().has(dedupKey(event, context))
}
