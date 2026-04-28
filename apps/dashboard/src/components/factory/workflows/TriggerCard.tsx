import { useState } from "react"
import { ChevronRight, GitBranch, Tag } from "lucide-react"
import { useIsMobile } from "@/hooks/use-mobile"
import { type GitHubAppInstallation } from "@/lib/api"
import { Button } from "@/components/ui/button"
import { Card, CardContent } from "@/components/ui/card"
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetFooter,
  SheetHeader,
  SheetTitle,
} from "@/components/ui/sheet"
import { LabelCombobox } from "./LabelCombobox"
import { RepoCombobox } from "./RepoCombobox"
import type { WorkflowTriggerDraft } from "./workflow-draft"

export interface TriggerCardProps {
  workspaceId: string
  installations: GitHubAppInstallation[]
  value: WorkflowTriggerDraft
  onChange: (next: WorkflowTriggerDraft) => void
}

export function TriggerCard({
  workspaceId,
  installations,
  value,
  onChange,
}: TriggerCardProps) {
  const [editing, setEditing] = useState(false)
  const isMobile = useIsMobile()

  const summary = summarizeTrigger(value)

  return (
    <>
      <Card
        role="button"
        aria-label="Edit trigger"
        tabIndex={0}
        className="cursor-pointer border-primary/40 transition hover:border-primary"
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
            <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-full bg-primary/10 text-primary">
              <Tag className="h-4 w-4" />
            </div>
            <div className="min-w-0 space-y-1">
              <div className="text-xs uppercase tracking-wide text-muted-foreground">
                Trigger · starts the flow
              </div>
              <div className="truncate text-sm font-medium">{summary.title}</div>
              <div className="truncate text-xs text-muted-foreground">
                {summary.detail}
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
            <SheetTitle>Edit trigger</SheetTitle>
            <SheetDescription>
              Pick the repo and label that starts the workflow.
            </SheetDescription>
          </SheetHeader>
          <div className="flex min-h-0 flex-1 flex-col gap-4 overflow-y-auto overscroll-contain px-4">
            <TriggerForm
              workspaceId={workspaceId}
              installations={installations}
              value={value}
              onChange={onChange}
            />
          </div>
          <SheetFooter>
            <Button className="w-full" size="lg" onClick={() => setEditing(false)}>
              Done
            </Button>
          </SheetFooter>
        </SheetContent>
      </Sheet>
    </>
  )
}

function TriggerForm({
  workspaceId,
  installations,
  value,
  onChange,
}: {
  workspaceId: string
  installations: GitHubAppInstallation[]
  value: WorkflowTriggerDraft
  onChange: (next: WorkflowTriggerDraft) => void
}) {
  return (
    <>
      <div className="space-y-1.5">
        <span className="text-sm font-medium">Repository</span>
        <RepoCombobox
          workspaceId={workspaceId}
          installations={installations}
          installId={value.install_id}
          repoOwner={value.repo_owner}
          repoName={value.repo_name}
          onChange={(next) =>
            onChange({
              ...value,
              install_id: next.install_id,
              repo_owner: next.repo_owner,
              repo_name: next.repo_name,
              label:
                next.install_id === value.install_id &&
                next.repo_owner === value.repo_owner &&
                next.repo_name === value.repo_name
                  ? value.label
                  : "",
            })
          }
        />
      </div>

      <div className="space-y-1.5">
        <span className="text-sm font-medium">Trigger label</span>
        {value.install_id && value.repo_owner && value.repo_name ? (
          <LabelCombobox
            workspaceId={workspaceId}
            installId={value.install_id}
            repoOwner={value.repo_owner}
            repoName={value.repo_name}
            value={value.label || null}
            onChange={(label) => onChange({ ...value, label })}
          />
        ) : (
          <p className="flex items-center gap-2 text-xs text-muted-foreground">
            <GitBranch className="h-3.5 w-3.5" />
            Pick a repository above.
          </p>
        )}
      </div>
    </>
  )
}

function summarizeTrigger(t: WorkflowTriggerDraft): {
  title: string
  detail: string
} {
  if (!t.install_id || !t.repo_owner || !t.repo_name) {
    return {
      title: "Pick a repository",
      detail: "GitHub issue label will start the workflow",
    }
  }
  if (!t.label) {
    return {
      title: `${t.repo_owner}/${t.repo_name}`,
      detail: "Pick a trigger label",
    }
  }
  return {
    title: `${t.repo_owner}/${t.repo_name}`,
    detail: `Label: ${t.label}`,
  }
}
