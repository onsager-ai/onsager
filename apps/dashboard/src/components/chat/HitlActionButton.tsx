import { type ReactNode, useCallback, useMemo, useState } from "react"
import { Button } from "@/components/ui/button"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { HitlCard } from "@/components/chat/HitlCard"
import type { HitlCardState } from "@/components/chat/hitl-types"
import { findMcpTool } from "@/lib/mcp-tools"
import { McpClientError, mcpCallTool } from "@/lib/mcp-client"

// Standalone HitlCard host for the noun surfaces (ADR 0025 / spec #514).
//
// ADR 0025 moves routine actions ("run a batch", "run this plan") off
// the chat thread and onto the noun surfaces, but HITL principle 1 still
// holds: every mutation routes through a HitlCard the user commits or
// rejects. ChatBuilder is the chat-side host; this is its page-button
// counterpart — a button that opens a dialog containing the same
// `HitlCard` primitive, then commits the call through the same
// `mcpCallTool` path. No new business logic: the card config comes from
// the shared `mcp-tools.ts` binding (`buildCard`), so the page button
// and the chat render an identical review surface for the same tool.

export interface HitlActionDialogProps {
  /** Whether the review dialog is open. */
  open: boolean
  onOpenChange: (open: boolean) => void
  /** MCP mutation tool to invoke. Must carry a `buildCard` binding. */
  toolName: string
  /** Proposed tool arguments; HitlCard edits are merged in at commit. */
  args: Record<string, unknown>
  /** Called with the tool's result text after a committed success. */
  onCommitted?: (resultText: string | undefined) => void
}

/**
 * Controlled review dialog — open/close owned by the caller. Use this
 * directly when the trigger is not a plain button (e.g. a dropdown menu
 * item); use {@link HitlActionButton} for the common button case.
 */
export function HitlActionDialog({
  open,
  onOpenChange,
  toolName,
  args,
  onCommitted,
}: HitlActionDialogProps) {
  const binding = findMcpTool(toolName)
  // `args` is a fresh object each render; key the memo on its serialized
  // form so the card only rebuilds when the proposed values change.
  const card = useMemo(
    () => binding?.buildCard?.(args),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [binding, toolName, JSON.stringify(args)],
  )

  const [state, setState] = useState<HitlCardState>("pending")
  const [errorMessage, setErrorMessage] = useState<string | undefined>(undefined)

  const handleOpenChange = useCallback(
    (next: boolean) => {
      onOpenChange(next)
      if (!next) {
        // Reset to pending so the next open starts fresh, not on a
        // stale committed/failed state from the prior run.
        setState("pending")
        setErrorMessage(undefined)
      }
    },
    [onOpenChange],
  )

  const handleCommit = useCallback(
    async (edits: Record<string, string>) => {
      setState("committing")
      setErrorMessage(undefined)
      const merged = { ...args, ...edits }
      try {
        const result = await mcpCallTool(toolName, merged)
        if (result.isError) {
          setState("failed")
          setErrorMessage(result.content[0]?.text ?? "tool returned isError=true")
          return
        }
        setState("committed")
        onCommitted?.(result.content[0]?.text)
        // Close once committed — the noun surface (new run, refreshed
        // list) is the confirmation, not a lingering dialog.
        handleOpenChange(false)
      } catch (err) {
        setState("failed")
        setErrorMessage(errorText(err))
      }
    },
    [args, toolName, onCommitted, handleOpenChange],
  )

  return (
    <Dialog open={open} onOpenChange={handleOpenChange}>
      <DialogContent className="sm:max-w-md">
        {/* The HitlCard carries the visible title; the dialog header is
            screen-reader-only so the primitive owns the visual heading. */}
        <DialogHeader className="sr-only">
          <DialogTitle>{card?.title ?? "Confirm action"}</DialogTitle>
          <DialogDescription>
            Review this action and confirm before it runs.
          </DialogDescription>
        </DialogHeader>
        {card ? (
          <HitlCard
            card={card}
            state={state}
            errorMessage={errorMessage}
            onCommit={handleCommit}
            onReject={() => handleOpenChange(false)}
          />
        ) : (
          <p className="text-sm text-muted-foreground">
            This action can&apos;t be reviewed — `{toolName}` is not in the
            dashboard&apos;s tool registry.
          </p>
        )}
      </DialogContent>
    </Dialog>
  )
}

export interface HitlActionButtonProps {
  toolName: string
  args: Record<string, unknown>
  /** Button content (icon + label). */
  children: ReactNode
  onCommitted?: (resultText: string | undefined) => void
  variant?: "default" | "outline" | "ghost" | "destructive" | "secondary"
  size?: "sm" | "default" | "icon"
  className?: string
  disabled?: boolean
  /** Accessible name for the trigger button (required for icon-only). */
  ariaLabel?: string
}

/**
 * Button that opens a {@link HitlActionDialog} for one mutation tool.
 * The common case behind the "Run a batch" / "Run" affordances.
 */
export function HitlActionButton({
  toolName,
  args,
  children,
  onCommitted,
  variant = "default",
  size = "sm",
  className,
  disabled,
  ariaLabel,
}: HitlActionButtonProps) {
  const [open, setOpen] = useState(false)
  return (
    <>
      <Button
        type="button"
        variant={variant}
        size={size}
        className={className}
        disabled={disabled}
        aria-label={ariaLabel}
        onClick={() => setOpen(true)}
        data-slot="hitl-action-button"
        data-tool={toolName}
      >
        {children}
      </Button>
      <HitlActionDialog
        open={open}
        onOpenChange={setOpen}
        toolName={toolName}
        args={args}
        onCommitted={onCommitted}
      />
    </>
  )
}

function errorText(err: unknown): string {
  if (err instanceof McpClientError) return `MCP error (${err.code}): ${err.message}`
  if (err instanceof Error) return err.message
  return String(err)
}
