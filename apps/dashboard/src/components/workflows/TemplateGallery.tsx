import { FilePlus2 } from "lucide-react"
import {
  WORKFLOW_PRESETS,
  emptyDocument,
  type WorkflowDocument,
} from "./workflow-draft"

export interface TemplateGalleryProps {
  onChoose: (doc: WorkflowDocument) => void
}

/**
 * First screen of the create dialog (template-first): a small gallery of a
 * blank workflow plus every {@link WORKFLOW_PRESETS} entry. Picking one seeds
 * the draft and drops the user into the tabbed editor — presets are a one-shot
 * bootstrap that reshapes the whole document, not an editable field, so they
 * live here rather than inside a tab. Preset names are repo-derived, but the
 * repo isn't chosen yet at this point, so we blank the name and let the
 * Overview tab's placeholder guide naming.
 */
export function TemplateGallery({ onChoose }: TemplateGalleryProps) {
  const blank = emptyDocument()

  return (
    <div className="space-y-4">
      <div>
        <h3 className="text-sm font-semibold">Start with a template</h3>
        <p className="text-xs text-muted-foreground">
          Pick a starting point — you can tune the trigger, steps, and
          repositories next.
        </p>
      </div>
      <div className="grid gap-2 sm:grid-cols-2">
        <TemplateCard
          label="Blank workflow"
          description="Start from scratch with no steps."
          icon={<FilePlus2 className="h-4 w-4 text-muted-foreground" />}
          onClick={() => onChoose(blank)}
        />
        {WORKFLOW_PRESETS.map((preset) => (
          <TemplateCard
            key={preset.id}
            label={preset.label}
            description={preset.description}
            onClick={() => onChoose({ ...preset.build(blank.trigger), name: "" })}
          />
        ))}
      </div>
    </div>
  )
}

function TemplateCard({
  label,
  description,
  icon,
  onClick,
}: {
  label: string
  description: string
  icon?: React.ReactNode
  onClick: () => void
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className="flex flex-col items-start gap-1 rounded-lg border p-3 text-left transition hover:border-primary/40 hover:bg-muted/50 focus-visible:border-ring focus-visible:ring-3 focus-visible:ring-ring/50 focus-visible:outline-none"
    >
      <span className="flex items-center gap-2 text-sm font-medium">
        {icon}
        {label}
      </span>
      <span className="text-xs text-muted-foreground">{description}</span>
    </button>
  )
}
