import { Link } from "react-router-dom"
import { FolderGit2, GitBranch, Webhook, Zap } from "lucide-react"
import { useOptionalActiveWorkspace } from "@/lib/workspace"
import { type GitHubAppInstallation } from "@/lib/api"
import { Input } from "@/components/ui/input"
import { LabelCombobox } from "./LabelCombobox"
import { RepoMultiCombobox } from "./RepoMultiCombobox"
import { TriggerKindPicker } from "./TriggerKindPicker"
import {
  setTriggerRepos,
  triggerRepos,
  type WorkflowTriggerDraft,
} from "./workflow-draft"

export interface TriggerEditorProps {
  workspaceId: string
  installations: GitHubAppInstallation[]
  value: WorkflowTriggerDraft
  onChange: (next: WorkflowTriggerDraft) => void
}

/**
 * Right-pane editor for the trigger node. Edits apply live to the draft via
 * `onChange` — there is no per-node "Done"; the persistent master-detail pane
 * replaces the old slide-out sheet. The form body is unchanged from the
 * previous `TriggerCard` sheet (manual vs github-webhook branches).
 */
export function TriggerEditor({
  workspaceId,
  installations,
  value,
  onChange,
}: TriggerEditorProps) {
  const isManual = value.kind_tag === "manual"
  const workspace = useOptionalActiveWorkspace()

  return (
    <div className="flex flex-col gap-4">
      <header className="flex items-start gap-3">
        <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-full bg-primary/10 text-primary">
          {isManual ? <Zap className="h-4 w-4" /> : <Webhook className="h-4 w-4" />}
        </div>
        <div className="min-w-0">
          <h3 className="text-sm font-semibold">Trigger</h3>
          <p className="text-xs text-muted-foreground">
            {isManual
              ? "No automatic trigger — runs only when you fire it."
              : "Pick the repositories and label that start the workflow."}
          </p>
        </div>
      </header>

      <TriggerKindPicker
        kindTag={value.kind_tag}
        onKindChange={(kind_tag) => onChange({ ...value, kind_tag })}
      />

      {isManual ? (
        <>
          <div className="space-y-1.5">
            <label htmlFor="manual-trigger-name" className="text-sm font-medium">
              Button label{" "}
              <span className="font-normal text-muted-foreground">(optional)</span>
            </label>
            <Input
              id="manual-trigger-name"
              value={value.manual_name}
              onChange={(e) => onChange({ ...value, manual_name: e.target.value })}
              placeholder="Defaults to the workflow name"
            />
            <p className="text-xs text-muted-foreground">
              Labels the run button; fire it from the dashboard or{" "}
              <code>onsager trigger fire</code>. Defaults to the workflow name.
            </p>
          </div>

          {/* Repo-less workflows are workspace-scoped (#553): the run is
              handed every repo bound to the workspace and the agent clones
              what it needs. Per-workflow repo pinning is a backend
              follow-up (#566); for now this states the actual runtime
              behavior rather than offering a choice we can't honor. */}
          <div className="space-y-1.5">
            <span className="text-sm font-medium">Repositories</span>
            <div className="flex items-start gap-2 rounded-md border bg-muted/30 px-3 py-2 text-xs text-muted-foreground">
              <FolderGit2 className="mt-0.5 h-3.5 w-3.5 shrink-0" />
              <span>
                Runs against{" "}
                <span className="font-medium text-foreground">
                  any repository bound to this workspace
                </span>{" "}
                — the agent gets the whole set and clones what it needs.
                {workspace ? (
                  <>
                    {" "}
                    <Link
                      to={`/workspaces/${workspace.slug}/settings`}
                      className="underline underline-offset-2 hover:text-foreground"
                    >
                      Manage workspace repositories
                    </Link>
                    .
                  </>
                ) : null}
              </span>
            </div>
          </div>
        </>
      ) : (
        <>
          <div className="space-y-1.5">
            <span className="text-sm font-medium">Repositories</span>
            <RepoMultiCombobox
              workspaceId={workspaceId}
              installations={installations}
              value={triggerRepos(value)}
              onChange={(repos) => {
                const next = setTriggerRepos(value, repos)
                // Labels are fetched against the primary repo; drop the
                // selected label when the primary changes so we never carry a
                // label that doesn't exist on the new repo.
                const primaryChanged =
                  next.repo_owner !== value.repo_owner ||
                  next.repo_name !== value.repo_name ||
                  next.install_id !== value.install_id
                onChange(primaryChanged ? { ...next, label: "" } : next)
              }}
            />
            {triggerRepos(value).length > 1 && (
              <p className="flex items-start gap-2 rounded-md border bg-muted/30 px-3 py-2 text-xs text-muted-foreground">
                <FolderGit2 className="mt-0.5 h-3.5 w-3.5 shrink-0" />
                <span>
                  Per-workflow multi-repo binding (#566) isn&apos;t wired on the
                  backend yet — for now this workflow triggers on{" "}
                  <span className="font-medium text-foreground">
                    {value.repo_owner}/{value.repo_name}
                  </span>{" "}
                  (the primary). The rest are saved for when #566 lands.
                </span>
              </p>
            )}
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
      )}
    </div>
  )
}
