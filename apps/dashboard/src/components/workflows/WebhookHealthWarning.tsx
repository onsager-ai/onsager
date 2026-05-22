import { AlertTriangle } from "lucide-react"
import type { InstallationDeliveryHealth } from "@/lib/api"

interface Props {
  health: InstallationDeliveryHealth | undefined
  /**
   * `inline` — compact one-liner suitable for a list card.
   * `block` — a full callout block suitable for the workflow detail page.
   * Both share copy; layout differs.
   */
  variant?: "inline" | "block"
}

// Spec #120 item 3: amber warn when GitHub's last K=30 App deliveries to
// this installation include non-2xx responses. `checked = 0` is silent
// (no recent deliveries to grade) — the warning is only useful when we
// have signal.
export function WebhookHealthWarning({ health, variant = "inline" }: Props) {
  if (!health || health.checked === 0 || health.non_2xx === 0) return null

  const allFailing = health.non_2xx === health.checked
  const headline = allFailing
    ? `All ${health.checked} recent GitHub webhook deliveries failed`
    : `${health.non_2xx} of ${health.checked} recent GitHub webhook deliveries failed`
  const detail = health.last_non_2xx_status_code
    ? `Most recent failure: HTTP ${health.last_non_2xx_status_code}. Check the App's webhook URL — canonical path is /webhooks/github.`
    : "Check the App's webhook URL — canonical path is /webhooks/github."

  if (variant === "block") {
    return (
      <div
        className="rounded-md border border-amber-500/30 bg-amber-500/5 p-3"
        role="status"
      >
        <div className="flex items-start gap-2">
          <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0 text-amber-600 dark:text-amber-400" />
          <div className="space-y-1">
            <p className="text-xs font-semibold uppercase tracking-wide text-amber-700 dark:text-amber-400">
              {headline}
            </p>
            <p className="text-xs text-muted-foreground">{detail}</p>
          </div>
        </div>
      </div>
    )
  }

  return (
    <div
      className="flex items-start gap-1.5 text-amber-700 dark:text-amber-400"
      role="status"
      title={detail}
    >
      <AlertTriangle className="mt-0.5 h-3.5 w-3.5 shrink-0" />
      <span className="truncate">{headline}</span>
    </div>
  )
}
