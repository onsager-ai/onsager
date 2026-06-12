import { Link } from "react-router-dom"
import { FolderGit2 } from "lucide-react"
import { useOptionalActiveWorkspace } from "@/lib/workspace"
import { type GitHubAppInstallation } from "@/lib/api"
import { RepoMultiCombobox } from "./RepoMultiCombobox"
import {
  setTriggerRepos,
  triggerRepos,
  type WorkflowTriggerDraft,
} from "./workflow-draft"

export interface TriggerReposEditorProps {
  workspaceId: string
  installations: GitHubAppInstallation[]
  value: WorkflowTriggerDraft
  onChange: (next: WorkflowTriggerDraft) => void
}

/**
 * "Repositories" tab content — *what a run operates on*. One home for every
 * trigger kind: repo-less kinds (manual, cron) are workspace-scoped — the run
 * gets every repo bound to the workspace — while a GitHub webhook trigger pins
 * the repo set the label fires against. Splitting repos out of the trigger
 * form lines up with #566 making repos a per-workflow binding rather than a
 * per-trigger field.
 */
export function TriggerReposEditor({
  workspaceId,
  installations,
  value,
  onChange,
}: TriggerReposEditorProps) {
  const workspace = useOptionalActiveWorkspace()

  // Repo-less workflows are workspace-scoped (#553): the run is handed every
  // repo bound to the workspace and the agent clones what it needs. Manual and
  // cron triggers both resolve repos this way at fire time.
  // Per-workflow repo pinning is a backend follow-up (#566).
  if (value.kind_tag === "manual" || value.kind_tag === "cron") {
    return (
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
    )
  }

  return (
    <div className="space-y-1.5">
      <span className="text-sm font-medium">Repositories</span>
      <RepoMultiCombobox
        workspaceId={workspaceId}
        installations={installations}
        value={triggerRepos(value)}
        onChange={(repos) => {
          const next = setTriggerRepos(value, repos)
          // Labels are fetched against the primary repo; drop the selected
          // label when the primary changes so we never carry a label that
          // doesn't exist on the new repo.
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
  )
}
