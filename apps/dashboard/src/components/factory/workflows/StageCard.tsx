import { useState } from "react"
import { ChevronRight, Trash2 } from "lucide-react"
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
import { ArtifactKindSelect } from "./ArtifactKindSelect"
import { GateKindToggle } from "./GateKindToggle"
import { GATE_KINDS } from "./workflow-meta"

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
            <div className="min-w-0">
              <div className="text-xs uppercase tracking-wide text-muted-foreground">
                Stage {index + 1} — {meta.label}
              </div>
              <div className="truncate text-sm font-medium">{stage.name}</div>
              <div className="truncate text-xs text-muted-foreground">
                on {stage.artifact_kind}
              </div>
            </div>
          </div>
          <ChevronRight className="h-4 w-4 shrink-0 text-muted-foreground" />
        </CardContent>
      </Card>
      <Sheet open={editing} onOpenChange={setEditing}>
        <SheetContent
          side={isMobile ? "bottom" : "right"}
          className={isMobile ? "rounded-t-xl" : ""}
        >
          <SheetHeader>
            <SheetTitle>Edit stage</SheetTitle>
            <SheetDescription>
              Pick a gate kind and what it operates on.
            </SheetDescription>
          </SheetHeader>
          <div className="flex flex-1 flex-col gap-4 overflow-y-auto px-4">
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
              <span className="text-sm font-medium">Artifact kind</span>
              <ArtifactKindSelect
                id={`stage-kind-${stage.id}`}
                value={stage.artifact_kind}
                onChange={(artifact_kind) => onChange({ ...stage, artifact_kind })}
              />
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
