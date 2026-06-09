import { Trash2 } from "lucide-react"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import type { WorkflowGateKind, WorkflowStage } from "@/lib/api"
import { ArtifactBadge } from "./ArtifactBadge"
import { ArtifactKindSelect } from "./ArtifactKindSelect"
import { GateKindToggle } from "./GateKindToggle"
import { defaultStageName } from "./workflow-draft"
import { GATE_KINDS, outputArtifactKind } from "./workflow-meta"

const GATE_LABEL = Object.fromEntries(
  GATE_KINDS.map((g) => [g.value, g.label]),
) as Record<WorkflowGateKind, string>

export interface StageEditorProps {
  stage: WorkflowStage
  index: number
  onChange: (next: WorkflowStage) => void
  onRemove: () => void
}

/**
 * Right-pane editor for a single stage node. Edits apply live to the draft
 * via `onChange` (no per-node "Done"). The stage name is auto-derived from
 * the gate kind and stays an optional override: changing the gate renames an
 * untouched stage so the rail label tracks the gate until the user types
 * their own name.
 */
export function StageEditor({ stage, index, onChange, onRemove }: StageEditorProps) {
  const output = outputArtifactKind(stage.gate_kind, stage.artifact_kind)
  const transforms = output !== stage.artifact_kind

  // Keep the name in lock-step with the gate while it's still the default,
  // so the rail reads "Governance" the instant you switch a fresh stage to
  // Governance — but never clobber a name the user actually typed.
  const changeGate = (gate_kind: WorkflowGateKind) => {
    const untouched =
      stage.name.trim() === "" || stage.name === defaultStageName(stage.gate_kind)
    onChange({
      ...stage,
      gate_kind,
      name: untouched ? defaultStageName(gate_kind) : stage.name,
    })
  }

  return (
    <div className="flex flex-col gap-4">
      <header>
        <div className="text-xs uppercase tracking-wide text-muted-foreground">
          Step {index + 1}
        </div>
        <h3 className="text-sm font-semibold">{GATE_LABEL[stage.gate_kind]}</h3>
      </header>

      <div className="space-y-1.5">
        <span className="text-sm font-medium">Gate kind</span>
        <GateKindToggle value={stage.gate_kind} onChange={changeGate} />
      </div>

      <div className="space-y-1.5">
        <div className="flex flex-wrap items-center justify-between gap-2">
          <span className="text-sm font-medium">Input artifact</span>
          {transforms && (
            <span className="inline-flex items-center gap-1 text-xs text-muted-foreground">
              produces
              <ArtifactBadge kind={output} size="sm" />
            </span>
          )}
        </div>
        <ArtifactKindSelect
          id={`stage-kind-${stage.id}`}
          value={stage.artifact_kind}
          onChange={(artifact_kind) => onChange({ ...stage, artifact_kind })}
        />
        <p className="text-xs text-muted-foreground">
          {transforms
            ? "This stage reads the input artifact and produces a new one."
            : "This stage inspects the artifact and passes it through."}
        </p>
      </div>

      <div className="space-y-1.5">
        <label htmlFor={`stage-name-${stage.id}`} className="text-sm font-medium">
          Name <span className="font-normal text-muted-foreground">(optional)</span>
        </label>
        <Input
          id={`stage-name-${stage.id}`}
          value={stage.name}
          onChange={(e) => onChange({ ...stage, name: e.target.value })}
          placeholder={defaultStageName(stage.gate_kind)}
        />
        <p className="text-xs text-muted-foreground">
          Defaults to the gate kind. Override it to label what this step does.
        </p>
      </div>

      <Button
        variant="outline"
        size="sm"
        className="self-start text-destructive hover:text-destructive"
        onClick={onRemove}
      >
        <Trash2 className="h-4 w-4" />
        Remove step
      </Button>
    </div>
  )
}
