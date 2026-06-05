import { useQuery } from "@tanstack/react-query"
import { api, type TriggerManifestEntry } from "@/lib/api"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"

// Kinds the builder can author today (#561). The manifest carries every
// registry kind, but the builder only knows how to collect config for
// these two; the rest (cron, interval, the GitHub webhook variants, …)
// land as the per-kind forms are built out. Order here is the display
// order in the selector.
const SELECTABLE_KINDS = ["github_issue_webhook", "manual"] as const

const FALLBACK: TriggerManifestEntry[] = [
  {
    kind_tag: "github_issue_webhook",
    producer: "portal",
    category: "request",
    ui_kind: "webhook",
    description: "Fires when a GitHub issue is labeled with the configured label.",
  },
  {
    kind_tag: "manual",
    producer: "portal",
    category: "manual",
    ui_kind: "manual",
    description: "Fires from a UI button or `onsager trigger fire` CLI command.",
  },
]

/**
 * Selector above the trigger form for the active trigger kind, sourced
 * from `/api/registry/triggers` (spec #237). Selecting a kind updates the
 * draft's `kind_tag`; the kind's manifest description stays visible below
 * the control. The builder offers the kinds it can collect config for
 * (`SELECTABLE_KINDS`); #561 adds Manual to the original webhook-only set.
 */
export function TriggerKindPicker({
  kindTag,
  onKindChange,
}: {
  kindTag: string
  onKindChange: (kindTag: string) => void
}) {
  const { data } = useQuery({
    queryKey: ["registry", "triggers"],
    queryFn: () => api.listTriggerManifest(),
    // The manifest is static at the binary level; cache it for the
    // session and don't refetch on every focus.
    staleTime: Infinity,
  })

  const all = data?.triggers ?? FALLBACK
  // Keep only the kinds the builder can author, in `SELECTABLE_KINDS`
  // order. Falls back to any single matching row when the manifest is
  // missing one (defensive — both rows exist server-side).
  const options = SELECTABLE_KINDS.map((tag) =>
    all.find((t) => t.kind_tag === tag),
  ).filter((t): t is TriggerManifestEntry => t !== undefined)

  const entry =
    options.find((t) => t.kind_tag === kindTag) ?? options[0] ?? all[0]
  if (!entry) return null

  return (
    <div className="space-y-1.5">
      <span className="text-xs uppercase tracking-wide text-muted-foreground">
        Trigger kind
      </span>
      <Select
        value={entry.kind_tag}
        onValueChange={(next) => {
          if (next) onKindChange(next)
        }}
      >
        <SelectTrigger className="w-full" aria-label="Trigger kind">
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          {options.map((t) => (
            <SelectItem key={t.kind_tag} value={t.kind_tag}>
              {humanLabel(t.kind_tag)}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
      <p className="text-xs text-muted-foreground">{entry.description}</p>
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
