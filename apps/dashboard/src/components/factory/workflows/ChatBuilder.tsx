import { type FormEvent, useState } from "react"
import { Send, Sparkles } from "lucide-react"
import { Button } from "@/components/ui/button"
import { Card, CardContent } from "@/components/ui/card"
import { Textarea } from "@/components/ui/textarea"
import {
  applyProposal,
  proposePlaceholder,
  type ProposeWorkflowCall,
} from "./propose-workflow"
import type { WorkflowDraft } from "./workflow-draft"

export interface ChatBuilderProps {
  draft: WorkflowDraft
  onChange: (next: WorkflowDraft) => void
}

interface ChatTurn {
  id: string
  prompt: string
  call: ProposeWorkflowCall
}

/**
 * Minimal chat builder. Stubs out the LLM round-trip — the structured tool
 * call is what matters for the spec. When the real agent is wired in, the
 * `proposePlaceholder` function is the single seam to replace.
 */
export function ChatBuilder({ draft, onChange }: ChatBuilderProps) {
  const [prompt, setPrompt] = useState("")
  const [turns, setTurns] = useState<ChatTurn[]>([])

  const onSubmit = (e: FormEvent) => {
    e.preventDefault()
    const text = prompt.trim()
    if (!text) return
    const call = proposePlaceholder(text)
    const next = applyProposal(draft, call)
    onChange(next)
    setTurns((prev) => [
      ...prev,
      { id: `${Date.now()}`, prompt: text, call },
    ])
    setPrompt("")
  }

  return (
    <Card>
      <CardContent className="flex flex-col gap-3 p-4">
        <div className="flex items-center gap-2 text-xs text-muted-foreground">
          <Sparkles className="h-3.5 w-3.5" />
          Describe the workflow. Updates land in the cards below.
        </div>
        {turns.length > 0 && (
          <ul className="space-y-2 text-sm">
            {turns.slice(-3).map((t) => (
              <li key={t.id} className="rounded-md border bg-muted/30 p-2">
                <div className="truncate">{t.prompt}</div>
                <div className="mt-1 text-xs text-muted-foreground">
                  Applied: {describeCall(t.call)}
                </div>
              </li>
            ))}
          </ul>
        )}
        <form onSubmit={onSubmit} className="flex flex-col gap-2">
          <Textarea
            value={prompt}
            onChange={(e) => setPrompt(e.target.value)}
            placeholder="e.g. Issue → agent → CI → manual merge"
            rows={2}
            aria-label="Describe the workflow"
          />
          <Button type="submit" disabled={!prompt.trim()} className="self-end">
            <Send className="h-4 w-4" />
            Propose
          </Button>
        </form>
      </CardContent>
    </Card>
  )
}

function describeCall(c: ProposeWorkflowCall): string {
  if (!c.stages || c.stages.length === 0) return "no changes"
  return c.stages.map((s) => s.gate_kind).join(" → ")
}
