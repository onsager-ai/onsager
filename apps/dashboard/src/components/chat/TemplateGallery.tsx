import { Card, CardContent } from "@/components/ui/card"
import { Badge } from "@/components/ui/badge"
import { TEMPLATES, type FtueTemplate } from "@/lib/templates"

interface TemplateGalleryProps {
  onPick: (template: FtueTemplate) => void
  selectedId?: string
}

// Horizontal scroll-strip of v0 templates (spec #406). Card click
// populates the right-panel DAG preview. Each card's subtitle is the
// `factory_framing` string — that's the surface #408 location 3
// commits to. (The chat empty state also carries the location-1
// inspection-report callout in `ChatPage.tsx`; this component owns
// the card surface, not the full empty-state vocabulary audit.)
export function TemplateGallery({ onPick, selectedId }: TemplateGalleryProps) {
  return (
    <div className="w-full">
      <div className="mb-2 flex items-baseline justify-between px-1">
        <h3 className="text-sm font-medium">Start from a template</h3>
        <span className="text-xs text-muted-foreground">
          {TEMPLATES.length} templates
        </span>
      </div>
      <ul
        className="flex w-full snap-x snap-mandatory list-none gap-3 overflow-x-auto pb-2 pl-1 pr-4"
        aria-label="Workflow templates"
      >
        {TEMPLATES.map((template) => (
          <li key={template.id} className="shrink-0 snap-start">
            <TemplateCard
              template={template}
              selected={selectedId === template.id}
              onClick={() => onPick(template)}
            />
          </li>
        ))}
      </ul>
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
      role="button"
      size="sm"
      data-selected={selected || undefined}
      className="w-64 cursor-pointer text-left transition-colors hover:bg-muted/40 data-[selected]:ring-2 data-[selected]:ring-primary"
      onClick={onClick}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault()
          onClick()
        }
      }}
      tabIndex={0}
      aria-pressed={selected}
      aria-label={`${template.name}. ${template.factory_framing}`}
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
        {template.id === "onsager-dogfood" ? (
          // Spec #407 link-surfacing — the dogfood template card carries
          // a "see live" pointer to the public showcase so an evaluator
          // can confirm the template isn't aspirational.
          <a
            href="/showcase/dogfood"
            onClick={(e) => e.stopPropagation()}
            className="text-[11px] text-primary underline-offset-4 hover:underline"
          >
            See this template running on Onsager itself →
          </a>
        ) : null}
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
