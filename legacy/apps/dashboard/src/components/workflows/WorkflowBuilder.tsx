import { Fragment, useState } from "react"
import { useMutation, useQueryClient } from "@tanstack/react-query"
import { useNavigate } from "react-router-dom"
import { ArrowRight, ChevronLeft } from "lucide-react"
import { api, type CreateWorkflowRequest, type GitHubAppInstallation } from "@/lib/api"
import { useOptionalActiveWorkspace } from "@/lib/workspace"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Switch } from "@/components/ui/switch"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { StepsEditor } from "./StepsEditor"
import { TemplateGallery } from "./TemplateGallery"
import { TriggerEventEditor } from "./TriggerEventEditor"
import { TriggerReposEditor } from "./TriggerReposEditor"
import {
  documentToCreateRequest,
  emptyDocument,
  isTriggerReady,
  type WorkflowDocument,
} from "./workflow-draft"

export interface WorkflowBuilderProps {
  workspaceId: string
  installations: GitHubAppInstallation[]
  initialDraft?: WorkflowDocument
  onCreated?: (id: string) => void
}

type TabId = "overview" | "trigger" | "steps" | "repos"

export function WorkflowBuilder({
  workspaceId,
  installations,
  initialDraft,
  onCreated,
}: WorkflowBuilderProps) {
  const [draft, setDraft] = useState<WorkflowDocument>(
    initialDraft ?? emptyDocument(),
  )
  // Template-first: an explicit draft (e.g. handed in from chat) skips the
  // gallery and lands straight in the tabbed editor.
  const [started, setStarted] = useState(initialDraft !== undefined)
  const [tab, setTab] = useState<TabId>("overview")
  // Creation and activation are distinct acts; default OFF preserves today's
  // draft-by-default behavior (the old "Save as draft" path).
  const [active, setActive] = useState(false)
  const queryClient = useQueryClient()
  const navigate = useNavigate()
  const activeWorkspace = useOptionalActiveWorkspace()

  const isCron = draft.trigger.kind_tag === "cron"
  // Per-tab completion drives the dots on the tab strip and the footer hint.
  const overviewDone = draft.name.trim() !== ""
  // The Trigger tab carries the per-kind config (manual: nothing; cron: a
  // schedule; github webhook: a label), so its completion is exactly the
  // trigger's readiness. Repositories are no longer required by any authorable
  // kind (manual and cron resolve repos workspace-scoped at fire time), so the
  // repo set never gates save.
  const triggerDone = isTriggerReady(draft.trigger)
  const stepsDone = draft.stages.length > 0

  const canSave = overviewDone && triggerDone && stepsDone
  const hint = !overviewDone
    ? "Name your workflow"
    : !triggerDone
      ? isCron
        ? "Add a cron schedule"
        : "Configure the trigger"
      : !stepsDone
        ? "Add at least one step"
        : ""

  const create = useMutation({
    mutationFn: (body: CreateWorkflowRequest) => api.createWorkflow(body),
    onSuccess: ({ workflow }) => {
      queryClient.invalidateQueries({ queryKey: ["workflows"] })
      if (onCreated) onCreated(workflow.id)
      else if (activeWorkspace) {
        navigate(`/workspaces/${activeWorkspace.slug}/workflows/${workflow.id}`)
      } else {
        navigate(`/workflows/${workflow.id}`)
      }
    },
  })

  const save = () => {
    if (!canSave) return
    create.mutate(
      documentToCreateRequest(draft, installations, workspaceId, active),
    )
  }

  if (!started) {
    return (
      <div className="flex min-h-0 flex-1 flex-col">
        <div className="min-h-0 flex-1 overflow-y-auto overscroll-contain">
          <TemplateGallery
            onChoose={(doc) => {
              setDraft(doc)
              setTab("overview")
              setStarted(true)
            }}
          />
        </div>
      </div>
    )
  }

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <Tabs
        value={tab}
        onValueChange={(v) => setTab(v as TabId)}
        className="flex min-h-0 flex-1 flex-col"
      >
        <TabsList className="w-full">
          <TabsTrigger value="overview">
            Overview {!overviewDone && <Dot />}
          </TabsTrigger>
          <TabsTrigger value="trigger">
            Trigger {!triggerDone && <Dot />}
          </TabsTrigger>
          <TabsTrigger value="steps">Steps {!stepsDone && <Dot />}</TabsTrigger>
          <TabsTrigger value="repos">Repositories</TabsTrigger>
        </TabsList>

        <div className="min-h-0 flex-1 overflow-y-auto overscroll-contain pt-4">
          <TabsContent value="overview" className="space-y-4">
            <div className="space-y-1.5">
              <label htmlFor="workflow-name" className="text-sm font-medium">
                Workflow name
              </label>
              <Input
                id="workflow-name"
                value={draft.name}
                onChange={(e) => setDraft({ ...draft, name: e.target.value })}
                placeholder="e.g. Issue → PR pipeline"
              />
            </div>

            <label
              htmlFor="workflow-active"
              className="flex items-start gap-3 rounded-lg border p-3"
            >
              <Switch
                id="workflow-active"
                checked={active}
                onCheckedChange={setActive}
              />
              <span className="space-y-0.5">
                <span className="block text-sm font-medium">
                  Activate on create
                </span>
                <span className="block text-xs text-muted-foreground">
                  Off = saved as a draft you can activate later.
                </span>
              </span>
            </label>

            <PipelineSummary draft={draft} onJump={setTab} />
          </TabsContent>

          <TabsContent value="trigger">
            <TriggerEventEditor
              workspaceId={workspaceId}
              value={draft.trigger}
              onChange={(trigger) => setDraft({ ...draft, trigger })}
              onGoToRepos={() => setTab("repos")}
            />
          </TabsContent>

          <TabsContent value="steps">
            <StepsEditor draft={draft} onChange={setDraft} />
          </TabsContent>

          <TabsContent value="repos">
            <TriggerReposEditor
              workspaceId={workspaceId}
              installations={installations}
              value={draft.trigger}
              onChange={(trigger) => setDraft({ ...draft, trigger })}
            />
          </TabsContent>
        </div>
      </Tabs>

      {create.isError && (
        <p className="pt-2 text-sm text-destructive">
          {create.error instanceof Error ? create.error.message : "Failed to save"}
        </p>
      )}

      <div className="mt-4 flex shrink-0 items-center justify-between gap-3 border-t pt-4">
        <div className="flex min-w-0 items-center gap-2">
          <Button
            type="button"
            variant="ghost"
            size="sm"
            className="-ml-2 shrink-0 text-muted-foreground"
            onClick={() => setStarted(false)}
          >
            <ChevronLeft className="h-4 w-4" />
            Templates
          </Button>
          {hint && (
            <span className="truncate text-xs text-muted-foreground">{hint}</span>
          )}
        </div>
        <Button
          type="button"
          size="lg"
          className="shrink-0"
          disabled={!canSave || create.isPending}
          onClick={save}
        >
          {create.isPending ? "Creating…" : "Create workflow"}
        </Button>
      </div>
    </div>
  )
}

/** Amber "needs config" marker on an incomplete tab. */
function Dot() {
  return (
    <span
      aria-hidden
      className="ml-0.5 inline-block h-1.5 w-1.5 rounded-full bg-amber-500"
    />
  )
}

/** Read-only `Trigger → Step → Step…` overview; chips jump to their tab. */
function PipelineSummary({
  draft,
  onJump,
}: {
  draft: WorkflowDocument
  onJump: (tab: TabId) => void
}) {
  const triggerLabel =
    draft.trigger.kind_tag === "manual"
      ? "Manual trigger"
      : draft.trigger.kind_tag === "cron"
        ? draft.trigger.expression?.trim()
          ? `Cron: ${draft.trigger.expression}`
          : "Cron trigger"
        : draft.trigger.label.trim()
          ? `Label: ${draft.trigger.label}`
          : "GitHub trigger"

  return (
    <div className="rounded-lg border bg-muted/20 p-3">
      <div className="mb-2 text-xs font-medium text-foreground">Pipeline</div>
      <div className="flex flex-wrap items-center gap-1.5 text-xs">
        <Chip onClick={() => onJump("trigger")}>{triggerLabel}</Chip>
        {draft.stages.map((stage) => (
          <Fragment key={stage.id}>
            <ArrowRight className="h-3 w-3 shrink-0 text-muted-foreground" />
            <Chip onClick={() => onJump("steps")}>{stage.name}</Chip>
          </Fragment>
        ))}
        {draft.stages.length === 0 && (
          <>
            <ArrowRight className="h-3 w-3 shrink-0 text-muted-foreground" />
            <Chip onClick={() => onJump("steps")} muted>
              No steps yet
            </Chip>
          </>
        )}
      </div>
    </div>
  )
}

function Chip({
  children,
  onClick,
  muted,
}: {
  children: React.ReactNode
  onClick: () => void
  muted?: boolean
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={
        muted
          ? "rounded-md border border-dashed px-2 py-1 text-muted-foreground transition hover:border-primary/40 hover:text-foreground"
          : "rounded-md border bg-background px-2 py-1 font-medium transition hover:border-primary/40"
      }
    >
      {children}
    </button>
  )
}
