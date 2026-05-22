import { useQuery } from "@tanstack/react-query"
import { api, type TriggerManifestEntry } from "@/lib/api"

const FALLBACK: TriggerManifestEntry[] = [
  {
    kind_tag: "github_issue_webhook",
    producer: "stiglab",
    category: "event",
    ui_kind: "webhook",
    description: "Fires when a GitHub issue is labeled with the configured label.",
  },
]

/**
 * Read-only badge above the trigger form that names the active kind and
 * its description, sourced from `/api/registry/triggers` (spec #237). The
 * dashboard renders one picker today because the registry has one row;
 * future kinds (cron, interval, …) will append manifest rows and the
 * picker will become a real selector.
 */
export function TriggerKindPicker({
  kindTag,
}: {
  kindTag: string
}) {
  const { data } = useQuery({
    queryKey: ["registry", "triggers"],
    queryFn: () => api.listTriggerManifest(),
    // The manifest is static at the binary level; cache it for the
    // session and don't refetch on every focus.
    staleTime: Infinity,
  })

  const triggers = data?.triggers ?? FALLBACK
  const entry = triggers.find((t) => t.kind_tag === kindTag) ?? triggers[0]
  if (!entry) return null

  return (
    <div className="rounded-md border bg-muted/30 px-3 py-2">
      <div className="text-xs uppercase tracking-wide text-muted-foreground">
        Trigger kind
      </div>
      <div className="text-sm font-medium">{humanLabel(entry.kind_tag)}</div>
      <div className="text-xs text-muted-foreground">{entry.description}</div>
    </div>
  )
}

function humanLabel(kindTag: string): string {
  switch (kindTag) {
    case "github_issue_webhook":
      return "GitHub issue webhook"
    default:
      return kindTag
  }
}
