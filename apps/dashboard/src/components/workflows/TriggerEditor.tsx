import { Link } from "react-router-dom"
import { FolderGit2, GitBranch, Zap } from "lucide-react"
import { useOptionalActiveWorkspace } from "@/lib/workspace"
import { type GitHubAppInstallation } from "@/lib/api"
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
 * The trigger section of the workflow builder — how a workflow is invoked.
 * A compact kind selector (Tabs) over the kind-specific form (#574). Edits
 * apply live to the draft via `onChange`; there is no per-section "Done".
 *
 * Manual is the "no automatic trigger, run it yourself" default (#572): it
 * needs no config, so its body is just a note about workspace-scoped repos
 * — no button-label input, since the run button derives its label from the
 * workflow name at create time. The GitHub-webhook leg keeps the repo +
 * label pickers.
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
    <div className="space-y-3">
      <TriggerKindPicker
        kindTag={value.kind_tag}
        onKindChange={(kind_tag) => onChange({ ...value, kind_tag })}
      />

      {isManual ? (
        // Repo-less workflows are workspace-scoped (#553): the run is handed
        // every repo bound to the workspace and the agent clones what it
        // needs. Per-workflow repo pinning is a backend follow-up (#566); for
        // now this states the actual runtime behavior rather than offering a
        // choice we can't honor.
        <div className="space-y-1 rounded-md border bg-muted/30 px-3 py-2 text-xs text-muted-foreground">
          <p className="flex items-center gap-1.5 font-medium text-foreground">
            <Zap className="h-3.5 w-3.5 shrink-0" />
            No automatic trigger
          </p>
          <p>
            Run it yourself from the dashboard or <code>onsager trigger fire</code>.
            Runs against{" "}
            <span className="font-medium text-foreground">
              any repository bound to this workspace
            </span>
            {workspace ? (
              <>
                {" — "}
                <Link
                  to={`/workspaces/${workspace.slug}/settings`}
                  className="underline underline-offset-2 hover:text-foreground"
                >
                  manage repositories
                </Link>
              </>
            ) : null}
            .
          </p>
        </div>
      ) : (
        <div className="space-y-3">
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
        </div>
      )}
    </div>
  )
}
