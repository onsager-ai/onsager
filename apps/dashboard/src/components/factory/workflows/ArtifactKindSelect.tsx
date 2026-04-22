import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import type { WorkflowArtifactKind } from "@/lib/api"
import { WORKFLOW_ARTIFACT_KINDS } from "./workflow-meta"

// Mobile-facing artifact-kind selector. Custom (user-defined) artifact
// kinds are deliberately hidden — power users edit those on desktop or via
// backend config.
export interface ArtifactKindSelectProps {
  value: WorkflowArtifactKind
  onChange: (value: WorkflowArtifactKind) => void
  id?: string
}

export function ArtifactKindSelect({ value, onChange, id }: ArtifactKindSelectProps) {
  return (
    <Select
      value={value}
      onValueChange={(v) => {
        // Registry-backed kinds (#102) — accept any id the registry emits.
        if (typeof v === "string" && v.length > 0) onChange(v)
      }}
      items={WORKFLOW_ARTIFACT_KINDS}
    >
      <SelectTrigger id={id} className="w-full">
        <SelectValue />
      </SelectTrigger>
      <SelectContent>
        {WORKFLOW_ARTIFACT_KINDS.map((k) => (
          <SelectItem key={k.value} value={k.value}>
            {k.label}
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  )
}
