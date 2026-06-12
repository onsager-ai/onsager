import { useMemo, useState } from "react"
import { AlertTriangle, Check, Loader2, Pencil, X } from "lucide-react"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card"
import { Input } from "@/components/ui/input"
import { cn } from "@/lib/utils"
import type {
  HitlCard as HitlCardSpec,
  HitlCardState,
  HitlConstructiveBody,
  HitlDestructiveBody,
  HitlDiffBody,
  HitlFieldSpec,
} from "./hitl-types"

export interface HitlCardProps {
  card: HitlCardSpec
  state: HitlCardState
  /** Surface error string from the last commit attempt, when state === 'failed'. */
  errorMessage?: string
  /** Apply with the (possibly edited) values. */
  onCommit: (editedValues: Record<string, string>) => void
  onReject: () => void
}

/**
 * The HITL trust seam. One primitive renders all three card shapes
 * (`constructive` / `diff` / `destructive`) via internal slot
 * renderers. Edits live client-side until commit; reject discards.
 *
 * Lifecycle states: `pending` → `committing` → `committed` | `failed`.
 * `failed` keeps the user's edits and lets them retry. `rejected` and
 * `superseded` are terminal-collapsed states.
 */
export function HitlCard({
  card,
  state,
  errorMessage,
  onCommit,
  onReject,
}: HitlCardProps) {
  const initialEdits = useMemo(() => initialEditValues(card), [card])
  const [edits, setEdits] = useState<Record<string, string>>(initialEdits)
  const [typedConfirm, setTypedConfirm] = useState("")
  // Reset client-side edits when the parent swaps in a new card (e.g.
  // a re-proposed tool call for the same mounted HitlCard). React's
  // "store previous prop value, reset on change" pattern: cheaper than
  // an effect (no extra render) and avoids the `set-state-in-effect`
  // anti-pattern. Card lifecycle transitions (pending → committing →
  // failed → retry) keep the same `card` reference, so retry preserves
  // the user's in-progress edits.
  const [prevCard, setPrevCard] = useState(card)
  if (card !== prevCard) {
    setPrevCard(card)
    setEdits(initialEdits)
    setTypedConfirm("")
  }

  if (state === "committed" || state === "rejected" || state === "superseded") {
    return <CollapsedCard card={card} state={state} />
  }

  const isCommitting = state === "committing"
  const isFailed = state === "failed"
  const needsConfirm = card.confirmTyping !== undefined
  const confirmMet =
    !needsConfirm || typedConfirm.trim() === card.confirmTyping?.expectedValue.trim()
  const commitDisabled = isCommitting || !confirmMet

  return (
    <Card className="gap-3" data-slot="hitl-card" data-kind={card.kind}>
      <CardHeader className="px-4">
        <CardTitle className="flex items-center gap-2">
          <Badge variant={badgeVariantFor(card.kind)} className="uppercase">
            {card.kind}
          </Badge>
          <span className="truncate">{card.title}</span>
        </CardTitle>
        {card.summary ? (
          <div className="text-xs text-muted-foreground">{card.summary}</div>
        ) : null}
      </CardHeader>
      <CardContent className="flex flex-col gap-3">
        {card.kind === "constructive" ? (
          <ConstructiveBody
            body={card.body as HitlConstructiveBody}
            edits={edits}
            disabled={isCommitting}
            onChange={setEdits}
          />
        ) : null}
        {card.kind === "diff" ? (
          <DiffBody body={card.body as HitlDiffBody} />
        ) : null}
        {card.kind === "destructive" ? (
          <DestructiveBody
            body={card.body as HitlDestructiveBody}
            sideEffects={card.sideEffects}
            reversibility={card.reversibility}
          />
        ) : null}

        {needsConfirm ? (
          <ConfirmTyping
            promptLabel={card.confirmTyping!.promptLabel}
            value={typedConfirm}
            disabled={isCommitting}
            onChange={setTypedConfirm}
          />
        ) : null}

        {isFailed && errorMessage ? (
          <div
            role="alert"
            className="flex items-start gap-2 rounded-md border border-destructive/40 bg-destructive/5 p-2 text-xs text-destructive"
          >
            <AlertTriangle className="mt-0.5 h-3.5 w-3.5 shrink-0" />
            <span>{errorMessage}</span>
          </div>
        ) : null}

        <div className="flex flex-wrap items-center justify-end gap-2 pt-1">
          <Button
            type="button"
            variant="ghost"
            size="sm"
            disabled={isCommitting}
            onClick={onReject}
          >
            <X className="h-3.5 w-3.5" />
            {card.reject.label}
          </Button>
          <Button
            type="button"
            variant={card.commit.intent === "destructive" ? "destructive" : "default"}
            size="sm"
            disabled={commitDisabled}
            onClick={() => onCommit(edits)}
          >
            {isCommitting ? (
              <Loader2 className="h-3.5 w-3.5 animate-spin" />
            ) : (
              <Check className="h-3.5 w-3.5" />
            )}
            {card.commit.label}
          </Button>
        </div>
      </CardContent>
    </Card>
  )
}

// -----------------------------------------------------------------------------
// Slot renderers
// -----------------------------------------------------------------------------

interface ConstructiveBodyProps {
  body: HitlConstructiveBody
  edits: Record<string, string>
  disabled: boolean
  onChange: (next: Record<string, string>) => void
}

function ConstructiveBody({
  body,
  edits,
  disabled,
  onChange,
}: ConstructiveBodyProps) {
  return (
    <div
      data-slot="hitl-card-constructive"
      className="flex flex-col gap-2 sm:grid sm:grid-cols-[max-content_1fr] sm:items-center sm:gap-x-3 sm:gap-y-2"
    >
      {body.fields.map((f, idx) => (
        <ConstructiveField
          key={`${f.label}-${idx}`}
          field={f}
          value={f.editable && f.key ? edits[f.key] ?? f.value : f.value}
          disabled={disabled}
          onChange={(next) => {
            if (!f.editable || !f.key) return
            onChange({ ...edits, [f.key]: next })
          }}
        />
      ))}
    </div>
  )
}

interface ConstructiveFieldProps {
  field: HitlFieldSpec
  value: string
  disabled: boolean
  onChange: (next: string) => void
}

function ConstructiveField({
  field,
  value,
  disabled,
  onChange,
}: ConstructiveFieldProps) {
  return (
    <>
      <div className="text-xs font-medium text-muted-foreground">{field.label}</div>
      {field.editable ? (
        <Input
          aria-label={field.label}
          value={value}
          placeholder={field.placeholder}
          disabled={disabled}
          onChange={(e) => onChange(e.target.value)}
        />
      ) : (
        <div className="text-sm">{value || <Empty />}</div>
      )}
    </>
  )
}

function DiffBody({ body }: { body: HitlDiffBody }) {
  const keys = useMemo(() => {
    const k = new Set<string>([
      ...Object.keys(body.before),
      ...Object.keys(body.after),
    ])
    return Array.from(k)
  }, [body])
  return (
    <div data-slot="hitl-card-diff" className="flex flex-col gap-1 text-sm">
      {keys.map((k) => {
        const before = body.before[k]
        const after = body.after[k]
        const status = diffStatus(before, after)
        return (
          <div
            key={k}
            data-status={status}
            className={cn(
              "grid grid-cols-[max-content_1fr] gap-x-3 rounded-md border px-2 py-1.5",
              status === "added" &&
                "border-emerald-600/40 bg-emerald-500/5 text-emerald-700 dark:text-emerald-300",
              status === "removed" &&
                "border-destructive/40 bg-destructive/5 text-destructive",
              status === "modified" &&
                "border-amber-600/40 bg-amber-500/5 text-amber-700 dark:text-amber-300",
              status === "unchanged" && "border-border bg-transparent",
            )}
          >
            <div className="text-xs font-medium uppercase">
              {diffMarker(status)} {k}
            </div>
            <div className="text-sm">
              {before !== undefined && status !== "added" ? (
                <span className="text-muted-foreground line-through">{before}</span>
              ) : null}
              {before !== undefined && after !== undefined && status !== "unchanged" ? (
                <span className="px-1 text-muted-foreground">→</span>
              ) : null}
              {after !== undefined && status !== "removed" ? (
                <span>{after}</span>
              ) : null}
            </div>
          </div>
        )
      })}
      <div className="mt-1 flex items-center gap-1 text-xs text-muted-foreground">
        <Pencil className="h-3 w-3" />
        Edits to proposed values land via the chat composer; reject to discard.
      </div>
    </div>
  )
}

interface DestructiveBodyProps {
  body: HitlDestructiveBody
  sideEffects?: string[]
  reversibility?: string
}

function DestructiveBody({
  body,
  sideEffects,
  reversibility,
}: DestructiveBodyProps) {
  return (
    <div data-slot="hitl-card-destructive" className="flex flex-col gap-2 text-sm">
      <div>{body.info}</div>
      {sideEffects && sideEffects.length > 0 ? (
        <ul className="list-disc space-y-0.5 pl-5 text-xs text-muted-foreground">
          {sideEffects.map((s, i) => (
            <li key={i}>{s}</li>
          ))}
        </ul>
      ) : null}
      {reversibility ? (
        <div
          className={cn(
            "text-xs font-medium",
            reversibility.toLowerCase().startsWith("irreversible")
              ? "text-destructive"
              : "text-muted-foreground",
          )}
        >
          {reversibility}
        </div>
      ) : null}
    </div>
  )
}

interface ConfirmTypingProps {
  promptLabel: string
  value: string
  disabled: boolean
  onChange: (next: string) => void
}

function ConfirmTyping({
  promptLabel,
  value,
  disabled,
  onChange,
}: ConfirmTypingProps) {
  return (
    <div className="flex flex-col gap-1">
      <div className="text-xs font-medium text-muted-foreground">{promptLabel}</div>
      <Input
        aria-label={promptLabel}
        value={value}
        disabled={disabled}
        onChange={(e) => onChange(e.target.value)}
      />
    </div>
  )
}

function CollapsedCard({
  card,
  state,
}: {
  card: HitlCardSpec
  state: HitlCardState
}) {
  const icon =
    state === "committed" ? (
      <Check className="h-3.5 w-3.5 text-emerald-600 dark:text-emerald-400" />
    ) : (
      <X className="h-3.5 w-3.5 text-muted-foreground" />
    )
  const text =
    state === "committed"
      ? `Committed: ${card.title}`
      : state === "rejected"
        ? `Rejected: ${card.title}`
        : `Superseded: ${card.title}`
  return (
    <div
      data-slot="hitl-card-collapsed"
      data-state={state}
      className="flex items-center gap-2 rounded-md border bg-muted/30 px-2 py-1.5 text-xs text-muted-foreground"
    >
      {icon}
      <span className="truncate">{text}</span>
    </div>
  )
}

// -----------------------------------------------------------------------------
// Small helpers
// -----------------------------------------------------------------------------

function Empty() {
  return <span className="text-muted-foreground italic">empty</span>
}

function initialEditValues(card: HitlCardSpec): Record<string, string> {
  const out: Record<string, string> = {}
  if (card.kind !== "constructive") return out
  const body = card.body as HitlConstructiveBody
  for (const f of body.fields) {
    if (f.editable && f.key) out[f.key] = f.value
  }
  return out
}

type DiffStatus = "added" | "removed" | "modified" | "unchanged"

function diffStatus(before: string | undefined, after: string | undefined): DiffStatus {
  if (before === undefined && after !== undefined) return "added"
  if (before !== undefined && after === undefined) return "removed"
  if (before !== after) return "modified"
  return "unchanged"
}

function diffMarker(status: DiffStatus): string {
  switch (status) {
    case "added":
      return "+"
    case "removed":
      return "−"
    case "modified":
      return "~"
    case "unchanged":
      return " "
  }
}

function badgeVariantFor(
  kind: HitlCardSpec["kind"],
): "default" | "secondary" | "destructive" | "outline" {
  switch (kind) {
    case "constructive":
      return "default"
    case "diff":
      return "secondary"
    case "destructive":
      return "destructive"
  }
}
