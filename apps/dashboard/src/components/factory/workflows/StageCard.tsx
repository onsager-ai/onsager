import { useState } from "react"
import { ArrowRight, ChevronRight, Trash2 } from "lucide-react"
import { useIsMobile } from "@/hooks/use-mobile"
import { Button } from "@/components/ui/button"
import { Card, CardContent } from "@/components/ui/card"
import { Input } from "@/components/ui/input"
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetFooter,
  SheetHeader,
  SheetTitle,
} from "@/components/ui/sheet"
import type { WorkflowStage } from "@/lib/api"
import { ArtifactBadge } from "./ArtifactBadge"
import { ArtifactKindSelect } from "./ArtifactKindSelect"
import { GateKindToggle } from "./GateKindToggle"
import { GATE_KINDS, outputArtifactKind } from "./workflow-meta"

// Single source of truth for gate kind metadata lives in
// `workflow-meta.ts`; derive per-gate lookups from there so the stage
// card can't drift out of sync with the toggle control.
const GATE_META = Object.fromEntries(
  GATE_KINDS.map((g) => [g.value, { icon: g.icon, label: g.label }]),
) as Record<
  WorkflowStage["gate_kind"],
  { icon: (typeof GATE_KINDS)[number]["icon"]; label: string }
>

export interface StageCardProps {
  stage: WorkflowStage
  index: number
  onChange: (next: WorkflowStage) => void
  onRemove: () => void
}

export function StageCard({ stage, index, onChange, onRemove }: StageCardProps) {
  const [editing, setEditing] = useState(false)
  const isMobile = useIsMobile()
  const meta = GATE_META[stage.gate_kind]
  const Icon = meta.icon
  const output = outputArtifactKind(stage.gate_kind, stage.artifact_kind)
  const transforms = output !== stage.artifact_kind

  return (
    <>
      <Card
        role="button"
        aria-label={`Edit stage ${stage.name}`}
        tabIndex={0}
        className="cursor-pointer transition hover:border-primary/40"
        onClick={() => setEditing(true)}
        onKeyDown={(e) => {
          if (e.key === "Enter" || e.key === " ") {
            e.preventDefault()
            setEditing(true)
          }
        }}
      >
        <CardContent className="flex items-center justify-between gap-3 p-4">
          <div className="flex min-w-0 items-center gap-3">
            <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-full bg-muted text-muted-foreground">
              <Icon className="h-4 w-4" />
            </div>
            <div className="min-w-0 space-y-1">
              <div className="text-xs uppercase tracking-wide text-muted-foreground">
                Stage {index + 1} — {meta.label}
              </div>
              <div className="truncate text-sm font-medium">{stage.name}</div>
              <div className="flex flex-wrap items-center gap-1.5">
                <span className="text-[10px] uppercase tracking-wide text-muted-foreground">
                  in
                </span>
                <ArtifactBadge kind={stage.artifact_kind} />
                {transforms && (
                  <>
                    <ArrowRight className="h-3 w-3 text-muted-foreground" />
                    <span className="text-[10px] uppercase tracking-wide text-muted-foreground">
                      out
                    </span>
                    <ArtifactBadge kind={output} />
                  </>
                )}
              </div>
            </div>
          </div>
          <ChevronRight className="h-4 w-4 shrink-0 text-muted-foreground" />
        </CardContent>
      </Card>
      <Sheet open={editing} onOpenChange={setEditing}>
        <SheetContent
          side={isMobile ? "bottom" : "right"}
          className={isMobile ? "h-[85dvh] rounded-t-xl" : "sm:max-w-lg"}
        >
          <SheetHeader>
            <SheetTitle>Edit stage</SheetTitle>
            <SheetDescription>
              Pick a gate kind and what it operates on.
            </SheetDescription>
          </SheetHeader>
          <div className="flex min-h-0 flex-1 flex-col gap-4 overflow-y-auto overscroll-contain px-4">
            <div className="space-y-1.5">
              <label htmlFor={`stage-name-${stage.id}`} className="text-sm font-medium">
                Name
              </label>
              <Input
                id={`stage-name-${stage.id}`}
                value={stage.name}
                onChange={(e) => onChange({ ...stage, name: e.target.value })}
                placeholder="What happens here"
              />
            </div>
            <div className="space-y-1.5">
              <span className="text-sm font-medium">Gate kind</span>
              <GateKindToggle
                value={stage.gate_kind}
                onChange={(gate_kind) => onChange({ ...stage, gate_kind })}
              />
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
          </div>
          <SheetFooter className="gap-2">
            <Button
              variant="outline"
              size="lg"
              className="w-full"
              onClick={() => {
                onRemove()
                setEditing(false)
              }}
            >
              <Trash2 className="h-4 w-4" />
              Remove stage
            </Button>
            <Button size="lg" className="w-full" onClick={() => setEditing(false)}>
              Done
            </Button>
          </SheetFooter>
        </SheetContent>
      </Sheet>
    </>
  )
}
