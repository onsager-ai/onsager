import { useQuery } from "@tanstack/react-query"
import { api, type TriggerManifestEntry } from "@/lib/api"
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs"

// Kinds the builder can author today (#561). The manifest carries every
// registry kind, but the builder only knows how to collect config for
// these two; the rest (cron, interval, the GitHub webhook variants, …)
// land as the per-kind forms are built out. Order here is the display
// order — Manual first, since it's the default (#572). The manifest
// (`/api/registry/triggers`) is still queried to satisfy the API
// contract, but the visible labels come from `humanLabel`.
const SELECTABLE_KINDS = ["manual", "github_issue_webhook"] as const

const FALLBACK: TriggerManifestEntry[] = [
  {
    kind_tag: "manual",
    producer: "portal",
    category: "manual",
    ui_kind: "manual",
    description: "Fires from a UI button or `onsager trigger fire` CLI command.",
  },
  {
    kind_tag: "github_issue_webhook",
    producer: "portal",
    category: "request",
    ui_kind: "webhook",
    description: "Fires when a GitHub issue is labeled with the configured label.",
  },
]

/**
 * Segmented control for the active trigger kind, sourced from
 * `/api/registry/triggers` (spec #237). Selecting a kind updates the
 * draft's `kind_tag`; the kind-specific form lives below in `TriggerEditor`.
 *
 * Rendered as Tabs rather than a Select (#574): with only two authorable
 * kinds, a segmented control is more compact and expressive, and it dodges
 * the Base-UI `Select.Value` default of rendering the raw `value`
 * (`github_issue_webhook`) instead of the human label.
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

  if (options.length === 0) return null
  // Highlight a known kind; fall back to the first option for an
  // unrecognized tag so the control never renders with nothing active.
  const active = options.some((t) => t.kind_tag === kindTag)
    ? kindTag
    : options[0].kind_tag

  return (
    <Tabs value={active} onValueChange={(v) => onKindChange(String(v))}>
      <TabsList className="w-full" aria-label="Trigger kind">
        {options.map((t) => (
          <TabsTrigger key={t.kind_tag} value={t.kind_tag} className="flex-1">
            {humanLabel(t.kind_tag)}
          </TabsTrigger>
        ))}
      </TabsList>
    </Tabs>
  )
}

function humanLabel(kindTag: string): string {
  switch (kindTag) {
    case "github_issue_webhook":
      return "GitHub issue"
    case "manual":
      return "Manual"
    default:
      return kindTag
  }
}
