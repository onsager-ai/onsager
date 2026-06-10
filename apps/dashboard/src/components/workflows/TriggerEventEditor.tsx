import { Clock, GitBranch, Zap } from "lucide-react"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { LabelCombobox } from "./LabelCombobox"
import { TriggerKindToggle } from "./TriggerKindToggle"
import { type WorkflowTriggerDraft } from "./workflow-draft"

export interface TriggerEventEditorProps {
  workspaceId: string
  value: WorkflowTriggerDraft
  onChange: (next: WorkflowTriggerDraft) => void
  /** Jump to the Repositories tab (the GitHub label needs a repo first). */
  onGoToRepos: () => void
}

/**
 * "Trigger" tab content — *what fires a run*: the trigger kind plus its event
 * config (a manual button name, or the GitHub issue label). The repo set
 * moved to its own "Repositories" tab (*what the run operates on*), so the
 * GitHub label picker reads the primary repo from the draft and, when none is
 * set yet, points the user at that tab instead of inlining a repo picker.
 */
export function TriggerEventEditor({
  workspaceId,
  value,
  onChange,
  onGoToRepos,
}: TriggerEventEditorProps) {
  const isManual = value.kind_tag === "manual"
  const isCron = value.kind_tag === "cron"
  const hasPrimaryRepo = Boolean(
    value.install_id && value.repo_owner && value.repo_name,
  )

  return (
    <div className="flex flex-col gap-4">
      <TriggerKindToggle
        kindTag={value.kind_tag}
        onKindChange={(kind_tag) => onChange({ ...value, kind_tag })}
      />

      {isManual ? (
        <div className="flex items-start gap-2 rounded-md border bg-muted/30 px-3 py-2 text-xs text-muted-foreground">
          <Zap className="mt-0.5 h-3.5 w-3.5 shrink-0" />
          <span>
            Fired on demand from the dashboard or{" "}
            <code>onsager trigger fire</code> — no label or repo required. The
            run button uses the workflow name.
          </span>
        </div>
      ) : isCron ? (
        <div className="flex flex-col gap-3">
          <div className="space-y-1.5">
            <label htmlFor="cron-expression" className="text-sm font-medium">
              Schedule
            </label>
            <Input
              id="cron-expression"
              value={value.expression ?? ""}
              onChange={(e) => onChange({ ...value, expression: e.target.value })}
              placeholder="0 9 * * 1-5"
              autoComplete="off"
              spellCheck={false}
              className="font-mono"
            />
            <p className="text-xs text-muted-foreground">
              A 5- or 6-field cron expression (e.g. <code>0 9 * * 1-5</code> ={" "}
              weekdays at 09:00). No repository required.
            </p>
          </div>
          <div className="space-y-1.5">
            <label htmlFor="cron-timezone" className="text-sm font-medium">
              Timezone <span className="text-muted-foreground">(optional)</span>
            </label>
            <Input
              id="cron-timezone"
              value={value.timezone ?? ""}
              onChange={(e) =>
                onChange({ ...value, timezone: e.target.value || null })
              }
              placeholder="UTC"
              autoComplete="off"
              spellCheck={false}
            />
            <p className="flex items-center gap-1.5 text-xs text-muted-foreground">
              <Clock className="h-3.5 w-3.5 shrink-0" />
              IANA timezone (e.g. <code>America/New_York</code>). Defaults to UTC.
            </p>
          </div>
        </div>
      ) : (
        <div className="space-y-1.5">
          <span className="text-sm font-medium">Trigger label</span>
          {hasPrimaryRepo ? (
            <LabelCombobox
              workspaceId={workspaceId}
              installId={value.install_id}
              repoOwner={value.repo_owner}
              repoName={value.repo_name}
              value={value.label || null}
              onChange={(label) => onChange({ ...value, label })}
            />
          ) : (
            <div className="flex items-center justify-between gap-2 rounded-md border bg-muted/30 px-3 py-2 text-xs text-muted-foreground">
              <span className="flex items-center gap-2">
                <GitBranch className="h-3.5 w-3.5 shrink-0" />
                Pick a repository first.
              </span>
              <Button
                type="button"
                variant="ghost"
                size="sm"
                onClick={onGoToRepos}
              >
                Repositories →
              </Button>
            </div>
          )}
          <p className="text-xs text-muted-foreground">
            The GitHub issue label that starts a run on the trigger
            repositories.
          </p>
        </div>
      )}
    </div>
  )
}
