import { useState } from "react"
import { X } from "lucide-react"
import { Button } from "@/components/ui/button"

// Location 5 of spec #408. First time a user lands on a bound workflow's
// detail page, show the factory-metaphor helper once. Dismissed state is
// global (not workspace-scoped, per spec #408 OQ): the metaphor is
// concept-level, learned once.
const STORAGE_KEY = "onsager.metaphor_seen.workflow_detail"

function readDismissed(): boolean {
  if (typeof window === "undefined") return true
  return window.localStorage.getItem(STORAGE_KEY) === "1"
}

export function MetaphorBanner() {
  const [dismissed, setDismissed] = useState<boolean>(readDismissed)

  const dismiss = () => {
    if (typeof window !== "undefined") {
      window.localStorage.setItem(STORAGE_KEY, "1")
    }
    setDismissed(true)
  }

  if (dismissed) return null

  return (
    <div
      role="note"
      className="flex items-start gap-3 rounded-md border bg-muted/30 px-3 py-2.5 text-sm"
    >
      <p className="flex-1 text-muted-foreground">
        This is your first production line. Each stage is a work station;
        each gate is a QC checkpoint. Once a run completes, you&apos;ll see
        the inspection reports here.
      </p>
      <Button
        type="button"
        variant="ghost"
        size="sm"
        onClick={dismiss}
        aria-label="Dismiss"
      >
        Got it
        <X className="h-3.5 w-3.5" />
      </Button>
    </div>
  )
}
