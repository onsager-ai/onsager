import { useQuery } from "@tanstack/react-query"
import { Clock, Webhook, Zap } from "lucide-react"
import { api, type TriggerManifestEntry } from "@/lib/api"
import { Button } from "@/components/ui/button"

// Kinds the builder can author today. The manifest carries every registry
// kind, but the builder only collects config for these; the rest (the other
// GitHub webhook variants, interval, …) land as the per-kind forms are built
// out. Manual is first so it's the default-visible kind. The GitHub issue
// webhook is hidden for now — still a valid wire/draft value, just not
// offered here yet. Order here is the display order in the toggle.
const SELECTABLE_KINDS = ["manual", "cron"] as const

const FALLBACK: TriggerManifestEntry[] = [
  {
    kind_tag: "manual",
    producer: "portal",
    category: "manual",
    ui_kind: "manual",
    description: "Fires from a UI button or `onsager trigger fire` CLI command.",
  },
  {
    kind_tag: "cron",
    producer: "substrate",
    category: "schedule",
    ui_kind: "cron",
    description: "Fires on a cron schedule (5- or 6-field expression, optional timezone).",
  },
]

const KIND_ICON: Record<string, typeof Webhook> = {
  github_issue_webhook: Webhook,
  manual: Zap,
  cron: Clock,
}

/**
 * Segmented control for the active trigger kind, mirroring {@link
 * GateKindToggle} (a row of `aria-pressed` toggle buttons, not an ARIA
 * radiogroup). Replaces the old `TriggerKindPicker` dropdown: with exactly
 * two authorable kinds a segmented control reads better and sidesteps the
 * Base-UI `Select.Value` default of rendering the raw `kind_tag` instead of
 * the human label. Descriptions stay manifest-sourced (`/api/registry/
 * triggers`, spec #237); `SELECTABLE_KINDS` scopes to the kinds we can
 * collect config for (#561 added Manual to the webhook-only set).
 */
export function TriggerKindToggle({
  kindTag,
  onKindChange,
}: {
  kindTag: string
  onKindChange: (kindTag: string) => void
}) {
  const { data } = useQuery({
    queryKey: ["registry", "triggers"],
    queryFn: () => api.listTriggerManifest(),
    // Static at the binary level; cache for the session.
    staleTime: Infinity,
  })

  const all = data?.triggers ?? FALLBACK
  const options = SELECTABLE_KINDS.map((tag) =>
    all.find((t) => t.kind_tag === tag),
  ).filter((t): t is TriggerManifestEntry => t !== undefined)
  if (options.length === 0) return null

  return (
    <div className="space-y-1.5">
      <span className="text-xs uppercase tracking-wide text-muted-foreground">
        Trigger kind
      </span>
      <div className="grid grid-cols-2 gap-2" role="group" aria-label="Trigger kind">
        {options.map((t) => {
          const active = t.kind_tag === kindTag
          const Icon = KIND_ICON[t.kind_tag] ?? Webhook
          return (
            <Button
              key={t.kind_tag}
              type="button"
              variant={active ? "default" : "outline"}
              aria-pressed={active}
              className="h-auto flex-col items-start gap-1 whitespace-normal px-3 py-2 text-left"
              onClick={() => onKindChange(t.kind_tag)}
            >
              <span className="flex items-center gap-2 text-sm font-medium">
                <Icon className="h-4 w-4" />
                {humanLabel(t.kind_tag)}
              </span>
              <span className="text-xs font-normal text-muted-foreground">
                {t.description}
              </span>
            </Button>
          )
        })}
      </div>
    </div>
  )
}

function humanLabel(kindTag: string): string {
  switch (kindTag) {
    case "github_issue_webhook":
      return "GitHub issue webhook"
    case "manual":
      return "Manual"
    default:
      return kindTag
  }
}
