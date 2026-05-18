// Spec #402 — the binding flow.
//
// One dialog launched from the right-panel `Bind to a repo →` button on
// ChatPage. Walks the user through up to three branches, skipping any
// whose prerequisite already exists:
//
//   A — Workspace (skipped if user has ≥ 1 workspace)
//   B — GitHub App install (skipped if chosen workspace has ≥ 1 install)
//   C — Project (always required; this is where bind actually happens)
//
// Step C calls `addWorkspaceProject` (no-op if the repo is already a
// project), submits the draft as a real workflow via `createWorkflow`,
// writes `bound_to` back into the draft, and navigates to the bound
// workflow's detail page. The user lands on the artifact they shipped,
// not back at the chat surface.

import {
  useEffect,
  useMemo,
  useState,
} from "react"
import { ArrowRight, GitBranch, Loader2 } from "lucide-react"
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query"
import { useNavigate } from "react-router-dom"

import {
  api,
  ApiError,
  type GitHubAppInstallation,
  type Project,
  type Workspace,
} from "@/lib/api"
import { markDraftBound } from "@/lib/drafts"
import { documentToCreateRequest } from "@/components/factory/workflows/workflow-draft"
import type { WorkflowDraft } from "@/components/factory/workflows/workflow-draft"
import { RepoCombobox } from "@/components/factory/workflows/RepoCombobox"
import { WorkspaceCreateForm } from "@/components/workspaces/NewWorkspaceDialog"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { Button, buttonVariants } from "@/components/ui/button"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { cn } from "@/lib/utils"

export type BindStep = "workspace" | "install" | "project"

interface BindDraftDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  /** The draft being bound. The dialog is a no-op without one. */
  draft: WorkflowDraft | null
  /** User id (for the draft's storage namespace). */
  userId: string | null
  /**
   * Optional initial workspace pre-selection — used by the "resume after
   * install" path so we land at Step C with the right workspace already
   * picked. The dialog still validates against the user's memberships.
   */
  initialWorkspaceId?: string | null
  /**
   * Optional initial step override. Used by the install-callback resume
   * path: caller passes "project" so the dialog skips the picker even if
   * the user has multiple workspaces.
   */
  initialStep?: BindStep
  /**
   * Called whenever the user advances a step. The host (ChatPage) can use
   * this to mutate the URL so post-install round-trips land back here in
   * the right place.
   */
  onStepChange?: (step: BindStep) => void
}

export function BindDraftDialog({
  open,
  onOpenChange,
  draft,
  userId,
  initialWorkspaceId,
  initialStep,
  onStepChange,
}: BindDraftDialogProps) {
  const queryClient = useQueryClient()
  const navigate = useNavigate()

  const { data: workspacesData, isLoading: workspacesLoading } = useQuery({
    queryKey: ["workspaces"],
    queryFn: api.listWorkspaces,
    enabled: open,
    staleTime: 30_000,
  })
  const workspaces = useMemo(
    () => workspacesData?.workspaces ?? [],
    [workspacesData],
  )

  // The selected workspace drives Steps B and C. `pickedId` is the
  // user's explicit choice (Step A's select, or a fresh `createWorkspace`
  // success); the resolved id below also folds in the caller's resume
  // hint and the auto-pick when exactly one workspace exists. Resolving
  // in render rather than an effect avoids the cascading-renders pattern
  // that react-hooks/set-state-in-effect flags.
  const [pickedId, setPickedId] = useState<string | null>(null)
  const workspaceId: string | null =
    pickedId ??
    (initialWorkspaceId && workspaces.some((w) => w.id === initialWorkspaceId)
      ? initialWorkspaceId
      : null) ??
    (workspaces.length === 1 ? workspaces[0].id : null)

  const selectedWorkspace = useMemo(
    () => workspaces.find((w) => w.id === workspaceId) ?? null,
    [workspaces, workspaceId],
  )

  const installsQuery = useQuery({
    queryKey: ["workspace-installations", workspaceId],
    queryFn: () => api.listWorkspaceInstallations(workspaceId!),
    enabled: open && !!workspaceId,
    staleTime: 30_000,
  })
  const installations = useMemo(
    () => installsQuery.data?.installations ?? [],
    [installsQuery.data],
  )
  const hasInstalls = installations.length > 0

  // Resolve the current step. Honour the caller's hint only on first
  // render — once the user moves forward, the natural prerequisite
  // ordering takes over.
  const [stepOverride, setStepOverride] = useState<BindStep | null>(
    initialStep ?? null,
  )
  const step: BindStep = useMemo(() => {
    if (!selectedWorkspace) return "workspace"
    if (!hasInstalls) return "install"
    if (stepOverride === "project") return "project"
    return "project"
  }, [selectedWorkspace, hasInstalls, stepOverride])

  useEffect(() => {
    if (!open) return
    onStepChange?.(step)
  }, [open, step, onStepChange])

  // Reset transient state when the dialog closes so the next open is
  // clean. We deliberately don't clear `workspaceId` on close — leaving
  // it sticky inside a session matches user expectation.
  const handleOpenChange = (next: boolean) => {
    if (!next) setStepOverride(null)
    onOpenChange(next)
  }

  if (!draft) {
    return (
      <Dialog open={open} onOpenChange={handleOpenChange}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Bind</DialogTitle>
            <DialogDescription>No draft to bind.</DialogDescription>
          </DialogHeader>
        </DialogContent>
      </Dialog>
    )
  }

  return (
    <Dialog open={open} onOpenChange={handleOpenChange}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>
            Bind {draft.name || draft.workflow.name || "Untitled draft"}
          </DialogTitle>
          <DialogDescription>
            {step === "workspace" &&
              "First, name your factory floor."}
            {step === "install" &&
              "Give Onsager access to your repos."}
            {step === "project" &&
              "Which repo runs this workflow?"}
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-4">
          {step === "workspace" && (
            <WorkspaceStep
              workspaces={workspaces}
              loading={workspacesLoading}
              onSelect={(ws) => {
                setPickedId(ws.id)
                queryClient.invalidateQueries({ queryKey: ["workspaces"] })
              }}
            />
          )}

          {step === "install" && selectedWorkspace && (
            <InstallStep
              workspace={selectedWorkspace}
              draftId={draft.id}
            />
          )}

          {step === "project" && selectedWorkspace && (
            <ProjectStep
              workspace={selectedWorkspace}
              installations={installations}
              draft={draft}
              userId={userId}
              onBound={(workflowId) => {
                onOpenChange(false)
                navigate(
                  `/workspaces/${selectedWorkspace.slug}/workflows/${workflowId}`,
                )
              }}
            />
          )}
        </div>

        <StepDots step={step} />
      </DialogContent>
    </Dialog>
  )
}

// ─── Step A: Workspace ──────────────────────────────────────────────────────

function WorkspaceStep({
  workspaces,
  loading,
  onSelect,
}: {
  workspaces: Workspace[]
  loading: boolean
  onSelect: (ws: Workspace) => void
}) {
  if (loading) {
    return (
      <p className="text-sm text-muted-foreground">Loading workspaces…</p>
    )
  }

  if (workspaces.length === 0) {
    return (
      <WorkspaceCreateForm
        onCreated={(ws) => onSelect(ws)}
        submitLabel="Create and continue →"
      />
    )
  }

  // ≥ 1 workspace exists but no pre-selection — render a picker. Per
  // spec, the dialog opens directly into selection in this case.
  return (
    <div className="space-y-2">
      <label className="text-sm font-medium" htmlFor="bind-workspace-select">
        Pick a workspace
      </label>
      <Select
        value=""
        onValueChange={(v) => {
          const ws = workspaces.find((w) => w.id === v)
          if (ws) onSelect(ws)
        }}
      >
        <SelectTrigger id="bind-workspace-select" className="w-full">
          <SelectValue placeholder="Pick a workspace" />
        </SelectTrigger>
        <SelectContent>
          {workspaces.map((w) => (
            <SelectItem key={w.id} value={w.id}>
              {w.name}{" "}
              <span className="ml-1 font-mono text-xs text-muted-foreground">
                {w.slug}
              </span>
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
    </div>
  )
}

// ─── Step B: GitHub App install ─────────────────────────────────────────────

function InstallStep({
  workspace,
  draftId,
}: {
  workspace: Workspace
  draftId: string
}) {
  // Round-trip target: come back to /chat with bind=continue so the
  // dialog re-opens at Step C. Portal honours `return_to` via the
  // install-callback cookie path (spec #402).
  const returnTo = `/chat?draft=${encodeURIComponent(draftId)}&bind=continue&workspace_id=${encodeURIComponent(workspace.id)}`
  const installUrl =
    `/api/github-app/install-start?workspace_id=${encodeURIComponent(workspace.id)}` +
    `&return_to=${encodeURIComponent(returnTo)}`

  return (
    <div className="space-y-3">
      <p className="text-sm text-muted-foreground">
        The Onsager GitHub App reads your repos and posts the workflows you
        bind. It does not read or post anywhere else.
      </p>
      <a href={installUrl} className={cn(buttonVariants(), "w-full")}>
        <GitBranch className="mr-1 h-3.5 w-3.5" />
        Install GitHub App
        <ArrowRight className="ml-1 h-3.5 w-3.5" />
      </a>
    </div>
  )
}

// ─── Step C: Project + bind ─────────────────────────────────────────────────

function ProjectStep({
  workspace,
  installations,
  draft,
  userId,
  onBound,
}: {
  workspace: Workspace
  installations: GitHubAppInstallation[]
  draft: WorkflowDraft
  userId: string | null
  onBound: (workflowId: string) => void
}) {
  const queryClient = useQueryClient()

  const [installId, setInstallId] = useState<string>("")
  const [repoOwner, setRepoOwner] = useState<string>("")
  const [repoName, setRepoName] = useState<string>("")
  const [error, setError] = useState<string | null>(null)

  const projectsQuery = useQuery({
    queryKey: ["workspace-projects", workspace.id],
    queryFn: () => api.listWorkspaceProjects(workspace.id),
    staleTime: 30_000,
  })
  const projects: Project[] = projectsQuery.data?.projects ?? []

  // The user has picked a repo via RepoCombobox once these three are set.
  const repoChosen = !!installId && !!repoOwner && !!repoName

  const bind = useMutation({
    mutationFn: async () => {
      // 1. Ensure a project exists for the chosen repo. Idempotent: if
      //    `addWorkspaceProject` returns a 409 (or similar) for an
      //    already-linked repo, look it up in the list we already
      //    queried. The backend returns the existing row's id either
      //    way, so we mostly care that the call doesn't block bind.
      const existing = projects.find(
        (p) =>
          p.github_app_installation_id === installId &&
          p.repo_owner === repoOwner &&
          p.repo_name === repoName,
      )
      if (!existing) {
        try {
          await api.addWorkspaceProject(workspace.id, {
            github_app_installation_id: installId,
            repo_owner: repoOwner,
            repo_name: repoName,
          })
        } catch (err) {
          // 409 / 422 likely means the repo is already a project under
          // a different code path; surface anything else.
          if (
            !(err instanceof ApiError) ||
            (err.status !== 409 && err.status !== 422)
          ) {
            throw err
          }
        }
      }

      // 2. Build the CreateWorkflowRequest from the draft, with the
      //    picked install/repo injected into the trigger config. We
      //    delegate to `documentToCreateRequest` so the same translator
      //    that powers WorkflowBuilder runs here — one path, one set of
      //    rules.
      const docWithTrigger = {
        ...draft.workflow,
        // Default name to the draft's display name if the document name
        // is blank (a fresh chat draft often has workflow.name === "").
        name: draft.workflow.name || draft.name || "Untitled workflow",
        trigger: {
          ...draft.workflow.trigger,
          install_id: installId,
          repo_owner: repoOwner,
          repo_name: repoName,
        },
      }
      const body = documentToCreateRequest(
        docWithTrigger,
        installations,
        workspace.id,
        true,
      )
      const { workflow } = await api.createWorkflow(body)

      // 3. Persist the binding back onto the draft. localStorage is
      //    best-effort — a quota failure here doesn't undo the bind.
      markDraftBound(userId, draft.id, workspace.id, workflow.id)

      return workflow
    },
    onSuccess: ({ id }) => {
      queryClient.invalidateQueries({ queryKey: ["workflows"] })
      queryClient.invalidateQueries({
        queryKey: ["workspace-projects", workspace.id],
      })
      onBound(id)
    },
    onError: (err) => {
      if (err instanceof ApiError) {
        setError(err.message || "Bind failed")
      } else if (err instanceof Error) {
        setError(err.message)
      } else {
        setError("Bind failed")
      }
    },
  })

  return (
    <div className="space-y-3">
      <div className="space-y-1.5">
        <label className="text-sm font-medium">Repository</label>
        <RepoCombobox
          workspaceId={workspace.id}
          installations={installations}
          installId={installId}
          repoOwner={repoOwner}
          repoName={repoName}
          onChange={({ install_id, repo_owner, repo_name }) => {
            setInstallId(install_id)
            setRepoOwner(repo_owner)
            setRepoName(repo_name)
            setError(null)
          }}
        />
      </div>

      {error ? <p className="text-xs text-destructive">{error}</p> : null}

      <Button
        type="button"
        className="w-full"
        disabled={!repoChosen || bind.isPending}
        onClick={() => {
          setError(null)
          bind.mutate()
        }}
      >
        {bind.isPending ? (
          <>
            <Loader2 className="mr-1 h-3.5 w-3.5 animate-spin" />
            Binding…
          </>
        ) : (
          <>
            Bind
            <ArrowRight className="ml-1 h-3.5 w-3.5" />
          </>
        )}
      </Button>
    </div>
  )
}

// ─── Step indicator ─────────────────────────────────────────────────────────

function StepDots({ step }: { step: BindStep }) {
  const order: BindStep[] = ["workspace", "install", "project"]
  const idx = order.indexOf(step)
  return (
    <div
      role="progressbar"
      aria-label="Binding progress"
      aria-valuenow={idx + 1}
      aria-valuemin={1}
      aria-valuemax={3}
      className="flex justify-center gap-1.5 pt-1 text-xs text-muted-foreground"
    >
      {order.map((s, i) => (
        <span
          key={s}
          aria-hidden
          className={cn(
            "inline-block h-1.5 w-1.5 rounded-full",
            i <= idx ? "bg-primary" : "bg-muted-foreground/30",
          )}
        />
      ))}
    </div>
  )
}
