import { Card, CardContent } from "@/components/ui/card"
import { Badge } from "@/components/ui/badge"
import { TEMPLATES, type FtueTemplate } from "@/lib/templates"

interface TemplateGalleryProps {
  onPick: (template: FtueTemplate) => void
  selectedId?: string
}

// Horizontal scroll-strip of v0 templates (spec #406). Card click
// populates the right-panel DAG preview. The factory-framing string
// lives here as the card subtitle — the only place the metaphor
// surfaces in the chat empty state (#408 location 3).
export function TemplateGallery({ onPick, selectedId }: TemplateGalleryProps) {
  return (
    <div className="w-full">
      <div className="mb-2 flex items-baseline justify-between px-1">
        <h3 className="text-sm font-medium">Start from a template</h3>
        <span className="text-xs text-muted-foreground">
          {TEMPLATES.length} templates
        </span>
      </div>
      <div
        className="flex w-full snap-x snap-mandatory gap-3 overflow-x-auto pb-2 pl-1 pr-4"
        role="list"
        aria-label="Workflow templates"
      >
        {TEMPLATES.map((template) => (
          <TemplateCard
            key={template.id}
            template={template}
            selected={selectedId === template.id}
            onClick={() => onPick(template)}
          />
        ))}
      </div>
    </div>
  )
}

function TemplateCard({
  template,
  selected,
  onClick,
}: {
  template: FtueTemplate
  selected: boolean
  onClick: () => void
}) {
  return (
    <Card
      role="listitem"
      size="sm"
      data-selected={selected || undefined}
      className="w-64 shrink-0 cursor-pointer snap-start text-left transition-colors hover:bg-muted/40 data-[selected]:ring-2 data-[selected]:ring-primary"
      onClick={onClick}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault()
          onClick()
        }
      }}
      tabIndex={0}
      aria-pressed={selected}
    >
      <CardContent className="flex flex-col gap-2">
        <div className="flex items-center gap-2">
          <h4 className="truncate text-sm font-semibold">{template.name}</h4>
          <Badge variant="outline" className="ml-auto shrink-0 text-[10px]">
            class {template.scenario_class}
          </Badge>
        </div>
        <p className="line-clamp-2 text-xs italic text-muted-foreground">
          {template.factory_framing}
        </p>
        <div className="text-[11px] text-muted-foreground/80">
          {template.stages.length} stage{template.stages.length !== 1 ? "s" : ""}
          {" · "}
          {triggerLabel(template.trigger_kind)}
        </div>
      </CardContent>
    </Card>
  )
}

function triggerLabel(triggerKind: string): string {
  switch (triggerKind) {
    case "github_issue_webhook":
      return "GitHub label"
    case "cron":
      return "Schedule"
    case "manual":
      return "Manual"
    default:
      return triggerKind
  }
}
