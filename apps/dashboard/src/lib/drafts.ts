// Client-side workflow-draft storage (spec #401).
//
// Drafts are a first-class user object the FTUE leans on: a P1 engineer
// who lands on `/chat` without a workspace must be able to design a
// workflow over multiple sessions, close the tab, reopen, and resume —
// all before the binding flow (axis 5) promotes the draft into a real
// spine workflow. The substrate stays untouched in v1; server-side draft
// sync is a v1.5 follow-up.

import { useCallback, useMemo, useState } from "react"

import type {
  WorkflowDraft,
  WorkflowDocument,
  WorkflowDraftSource,
} from "@/components/factory/workflows/workflow-draft"

/** Soft cap on stored drafts per user. Oldest by `updated_at` evicted. */
const DRAFT_CAP = 50

/** Anonymous-user slot — used if axis 1 Open Question 7 ever opens up. */
const ANON_USER_KEY = "anon"

function storageKey(userId: string | null | undefined): string {
  return `onsager.drafts.${userId && userId.length > 0 ? userId : ANON_USER_KEY}`
}

function newId(): string {
  if (typeof crypto !== "undefined" && "randomUUID" in crypto) {
    return crypto.randomUUID()
  }
  return `draft_${Math.random().toString(36).slice(2, 10)}_${Date.now()}`
}

function emptyDocument(): WorkflowDocument {
  return {
    name: "",
    trigger: { install_id: "", repo_owner: "", repo_name: "", label: "" },
    stages: [],
  }
}

/** Build a fresh draft record. */
export function makeDraft(
  userId: string | null | undefined,
  source: WorkflowDraftSource = "blank",
  workflow: WorkflowDocument = emptyDocument(),
  name = "Untitled draft",
  templateId?: string,
): WorkflowDraft {
  const now = new Date().toISOString()
  return {
    id: newId(),
    user_id: userId ?? ANON_USER_KEY,
    name,
    source,
    template_id: templateId,
    workflow,
    created_at: now,
    updated_at: now,
  }
}

interface DraftsBlob {
  drafts: WorkflowDraft[]
}

/** Read every persisted draft for the user, newest-first. */
export function loadDrafts(userId: string | null | undefined): WorkflowDraft[] {
  if (typeof window === "undefined") return []
  try {
    const raw = window.localStorage.getItem(storageKey(userId))
    if (!raw) return []
    const parsed: unknown = JSON.parse(raw)
    // Two on-disk shapes are accepted: `{drafts: [...]}` (current) and a
    // bare array (cheap to write, easy to migrate). Both deserialize to
    // a typed list with newest-first ordering.
    const list = Array.isArray(parsed)
      ? (parsed as WorkflowDraft[])
      : Array.isArray((parsed as DraftsBlob)?.drafts)
        ? (parsed as DraftsBlob).drafts
        : []
    return list
      .filter(isValidDraft)
      .sort((a, b) => (a.updated_at < b.updated_at ? 1 : -1))
  } catch {
    return []
  }
}

function isValidDraft(v: unknown): v is WorkflowDraft {
  if (typeof v !== "object" || v == null) return false
  const r = v as Record<string, unknown>
  return (
    typeof r.id === "string" &&
    typeof r.user_id === "string" &&
    typeof r.name === "string" &&
    typeof r.source === "string" &&
    typeof r.created_at === "string" &&
    typeof r.updated_at === "string" &&
    typeof r.workflow === "object" &&
    r.workflow != null
  )
}

function writeDrafts(
  userId: string | null | undefined,
  drafts: WorkflowDraft[],
): void {
  if (typeof window === "undefined") return
  try {
    window.localStorage.setItem(
      storageKey(userId),
      JSON.stringify({ drafts }),
    )
  } catch (err) {
    // Quota exhaustion / private mode — surface to console and move on.
    // The draft strip shows live state; persistence is best-effort.
    console.warn("[onsager] failed to persist workflow drafts:", err)
  }
}

/**
 * Upsert a draft. Truncates to `DRAFT_CAP` by `updated_at` (oldest evicted
 * silently per spec #401's Open Questions section). Returns the freshly
 * persisted list.
 */
export function saveDraft(
  userId: string | null | undefined,
  draft: WorkflowDraft,
): WorkflowDraft[] {
  const all = loadDrafts(userId)
  const next: WorkflowDraft = { ...draft, updated_at: new Date().toISOString() }
  const without = all.filter((d) => d.id !== next.id)
  let merged = [next, ...without]
  if (merged.length > DRAFT_CAP) {
    // Newest-first; drop the tail.
    merged = merged.slice(0, DRAFT_CAP)
  }
  writeDrafts(userId, merged)
  return merged
}

export function deleteDraft(
  userId: string | null | undefined,
  draftId: string,
): WorkflowDraft[] {
  const all = loadDrafts(userId)
  const next = all.filter((d) => d.id !== draftId)
  writeDrafts(userId, next)
  return next
}

export interface UseWorkflowDraftResult {
  /** The active draft. Null only before mount completes / after delete. */
  draft: WorkflowDraft | null
  /** All drafts owned by this user, newest first. */
  drafts: WorkflowDraft[]
  /**
   * Patch the active draft. `workflow` updates the inner document;
   * `name`/`source`/`template_id`/`bound_to` update the outer record.
   * Touches `updated_at`.
   */
  updateDraft: (patch: Partial<WorkflowDraft>) => void
  /** Replace the active draft's `workflow` document. */
  setWorkflow: (workflow: WorkflowDocument) => void
  /** Switch the active draft by id. Creates a new draft if id is null. */
  switchDraft: (id: string | null) => void
  /** Create a new draft and make it active. */
  newDraft: (
    source?: WorkflowDraftSource,
    workflow?: WorkflowDocument,
    name?: string,
    templateId?: string,
  ) => WorkflowDraft
  /** Delete a draft. If it was the active one, switches to the next newest. */
  deleteById: (id: string) => void
}

/**
 * Top-level draft hook. The active draft is held in component state so
 * downstream renders don't have to re-read localStorage; each mutation
 * persists via `saveDraft`.
 *
 * Pass `null` for `userId` before auth resolves; the hook keeps a
 * stable empty list until a real user id arrives.
 */
export function useWorkflowDraft(
  userId: string | null | undefined,
): UseWorkflowDraftResult {
  // `version` ticks on every mutation; combined with `userId` it drives
  // a derived `drafts` list via useMemo. This keeps state-as-cache out
  // of effects (React lint flags setState-in-effect) — `drafts` is
  // *the* source of truth, recomputed by mutation rather than mirrored.
  const [version, setVersion] = useState(0)
  const drafts = useMemo(
    () => loadDrafts(userId),
    // `version` is a cache-busting signal: each mutation in this hook
    // ticks it so the memo re-reads localStorage. `loadDrafts` doesn't
    // close over `version`, so React's exhaustive-deps lint thinks it's
    // unnecessary — but the explicit dependency is the whole point.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [userId, version],
  )

  // `activeId` is mutable selection state. Per-user reset (login swap)
  // happens implicitly: the memo above flips to the new user's list,
  // and `draft` (derived below) resolves to null until the caller calls
  // `switchDraft` or `newDraft`.
  const [activeId, setActiveId] = useState<string | null>(null)

  const bumpAndSelect = useCallback((id: string | null) => {
    setActiveId(id)
    setVersion((v) => v + 1)
  }, [])

  const newDraft = useCallback(
    (
      source: WorkflowDraftSource = "blank",
      workflow: WorkflowDocument = emptyDocument(),
      name = "Untitled draft",
      templateId?: string,
    ): WorkflowDraft => {
      const fresh = makeDraft(userId, source, workflow, name, templateId)
      saveDraft(userId, fresh)
      bumpAndSelect(fresh.id)
      return fresh
    },
    [userId, bumpAndSelect],
  )

  const switchDraft = useCallback(
    (id: string | null) => {
      if (id === null) {
        newDraft()
        return
      }
      setActiveId(id)
    },
    [newDraft],
  )

  const updateDraft = useCallback(
    (patch: Partial<WorkflowDraft>) => {
      // Read the *current* list at mutation time. The memo will refresh
      // on the next render when version ticks.
      const current = loadDrafts(userId)
      const active = current.find((d) => d.id === activeId)
      if (!active) return
      const merged: WorkflowDraft = { ...active, ...patch }
      saveDraft(userId, merged)
      setVersion((v) => v + 1)
    },
    [activeId, userId],
  )

  // `setWorkflow` is the path that propose_workflow / propose_workflow_draft
  // tool-call commits take in ChatPage. The FTUE flow can hit this before
  // the user has clicked a template or `+ New draft`, so the call must
  // auto-create a draft on first write rather than silently no-op.
  const setWorkflow = useCallback(
    (workflow: WorkflowDocument) => {
      const current = loadDrafts(userId)
      const targetId = activeId ?? current[0]?.id
      if (!targetId) {
        const fresh = makeDraft(
          userId,
          "chat",
          workflow,
          workflow.name || "Untitled draft",
        )
        saveDraft(userId, fresh)
        bumpAndSelect(fresh.id)
        return
      }
      const active = current.find((d) => d.id === targetId)
      if (!active) {
        const fresh = makeDraft(
          userId,
          "chat",
          workflow,
          workflow.name || "Untitled draft",
        )
        saveDraft(userId, fresh)
        bumpAndSelect(fresh.id)
        return
      }
      saveDraft(userId, { ...active, workflow })
      setVersion((v) => v + 1)
    },
    [activeId, userId, bumpAndSelect],
  )

  const deleteById = useCallback(
    (id: string) => {
      deleteDraft(userId, id)
      if (id === activeId) setActiveId(null)
      setVersion((v) => v + 1)
    },
    [activeId, userId],
  )

  // Default the active id to the newest draft once the list resolves —
  // computed in render, no effect needed. `useState`'s initial value
  // can't observe `drafts` (the memo depends on `userId`/`version` set
  // *here*), so we resolve at read time instead.
  const resolvedActiveId =
    activeId ?? (drafts.length > 0 ? drafts[0].id : null)
  const draft = drafts.find((d) => d.id === resolvedActiveId) ?? null

  return {
    draft,
    drafts,
    updateDraft,
    setWorkflow,
    switchDraft,
    newDraft,
    deleteById,
  }
}
