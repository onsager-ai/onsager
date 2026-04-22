import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import type { WorkflowArtifactKind } from "@/lib/api"
import { useWorkflowKinds } from "./useWorkflowKinds"

// Mobile-facing artifact-kind selector. The option list comes from the
// registry via `GET /api/workflow/kinds` (issue #102); `useWorkflowKinds`
// silently falls back to the static set baked into `workflow-meta.ts` if
// the fetch is loading or fails, so the builder stays usable offline.
export interface ArtifactKindSelectProps {
  value: WorkflowArtifactKind
  onChange: (value: WorkflowArtifactKind) => void
  id?: string
}

export function ArtifactKindSelect({ value, onChange, id }: ArtifactKindSelectProps) {
  const { kinds } = useWorkflowKinds()
  return (
    <Select
      value={value}
      onValueChange={(v) => {
        // Registry-backed kinds (#102) — accept any id the registry emits.
        if (typeof v === "string" && v.length > 0) onChange(v)
      }}
      items={kinds}
    >
      <SelectTrigger id={id} className="w-full">
        <SelectValue />
      </SelectTrigger>
      <SelectContent>
        {kinds.map((k) => (
          <SelectItem key={k.value} value={k.value}>
            {k.label}
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  )
}
