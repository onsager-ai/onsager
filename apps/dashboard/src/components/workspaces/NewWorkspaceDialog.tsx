import { useState, type FormEvent } from "react"
import { useMutation, useQueryClient } from "@tanstack/react-query"
import { api, ApiError, type Workspace } from "@/lib/api"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"

function slugify(value: string) {
  return value
    .toLowerCase()
    .replace(/[^a-z0-9-]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, 40)
}

/**
 * Modal for creating a new workspace. Auto-suggests a slug from the display
 * name until the user overrides it.
 */
export function NewWorkspaceDialog({
  open,
  onOpenChange,
  onCreated,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
  onCreated?: (id: string) => void
}) {
  const queryClient = useQueryClient()
  const [name, setName] = useState("")
  const [customSlug, setCustomSlug] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)

  // Auto-derive slug from the display name until the user edits the slug
  // field directly; then keep their override.
  const slug = customSlug ?? slugify(name)

  const reset = () => {
    setName("")
    setCustomSlug(null)
    setError(null)
  }

  const handleOpenChange = (next: boolean) => {
    if (!next) reset()
    onOpenChange(next)
  }

  const create = useMutation({
    mutationFn: () => api.createWorkspace({ slug, name }),
    onSuccess: (res) => {
      queryClient.invalidateQueries({ queryKey: ["workspaces"] })
      reset()
      onOpenChange(false)
      onCreated?.(res.workspace.id)
    },
    onError: (err) =>
      setError(err instanceof Error ? err.message : "Failed to create workspace"),
  })

  return (
    <Dialog open={open} onOpenChange={handleOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Create a workspace</DialogTitle>
          <DialogDescription>
            A workspace groups the GitHub projects and agent sessions that share
            credentials and governance. You can create more later.
          </DialogDescription>
        </DialogHeader>
        <form
          id="new-workspace-form"
          onSubmit={(e) => {
            e.preventDefault()
            if (create.isPending) return
            if (slug && name) create.mutate()
          }}
          className="space-y-3"
        >
          <label className="block space-y-1">
            <span className="text-sm font-medium">Display name</span>
            <Input
              placeholder="Acme Inc."
              value={name}
              onChange={(e) => setName(e.target.value)}
              autoFocus
            />
          </label>
          <label className="block space-y-1">
            <span className="text-sm font-medium">Slug</span>
            <Input
              placeholder="acme"
              value={slug}
              onChange={(e) => setCustomSlug(slugify(e.target.value))}
              className="font-mono"
            />
            <span className="text-xs text-muted-foreground">
              Lowercase letters, numbers, and hyphens. Used in URLs and APIs.
            </span>
          </label>
          {error && <p className="text-xs text-destructive">{error}</p>}
        </form>
        <DialogFooter>
          <Button
            type="button"
            variant="outline"
            onClick={() => handleOpenChange(false)}
          >
            Cancel
          </Button>
          <Button
            type="submit"
            form="new-workspace-form"
            disabled={!slug || !name || create.isPending}
          >
            {create.isPending ? "Creating…" : "Create workspace"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}

/**
 * Slimmed-down workspace-create surface used by the FTUE binding flow
 * (spec #402, Step A). Asks for the display name only; the slug is
 * auto-derived. Power-user surfaces (`/workspaces` Create button) keep
 * the full `NewWorkspaceDialog` with its explicit slug field.
 *
 * On success, invalidates the workspaces query so the parent picker
 * sees the new row, and calls `onCreated` with the persisted record so
 * the binding dialog can advance to Step B / C without re-querying.
 */
export function WorkspaceCreateForm({
  onCreated,
  submitLabel = "Create and continue →",
  autoFocus = true,
}: {
  onCreated: (workspace: Workspace) => void
  submitLabel?: string
  autoFocus?: boolean
}) {
  const queryClient = useQueryClient()
  const [name, setName] = useState("")
  const [error, setError] = useState<string | null>(null)

  const slug = slugify(name)

  const create = useMutation({
    mutationFn: () => api.createWorkspace({ slug, name }),
    onSuccess: (res) => {
      queryClient.invalidateQueries({ queryKey: ["workspaces"] })
      onCreated(res.workspace)
    },
    onError: (err) => {
      if (err instanceof ApiError) {
        setError(err.message || "Failed to create workspace")
      } else if (err instanceof Error) {
        setError(err.message)
      } else {
        setError("Failed to create workspace")
      }
    },
  })

  const canSubmit = !!name.trim() && !!slug && !create.isPending

  const onSubmit = (e: FormEvent) => {
    e.preventDefault()
    setError(null)
    if (!canSubmit) return
    create.mutate()
  }

  return (
    <form onSubmit={onSubmit} className="space-y-3">
      <label className="block space-y-1">
        <span className="text-sm font-medium">Workspace name</span>
        <Input
          placeholder="Personal"
          value={name}
          onChange={(e) => setName(e.target.value)}
          autoFocus={autoFocus}
          aria-label="Workspace name"
        />
        <span className="text-xs text-muted-foreground">
          e.g. <em>Acme Engineering</em> or your personal scope.
        </span>
      </label>
      {error ? <p className="text-xs text-destructive">{error}</p> : null}
      <Button type="submit" disabled={!canSubmit} className="w-full">
        {create.isPending ? "Creating…" : submitLabel}
      </Button>
    </form>
  )
}
