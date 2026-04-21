import { useState } from "react"
import { useQuery } from "@tanstack/react-query"
import { ChevronRight, GitBranch, Tag } from "lucide-react"
import { useIsMobile } from "@/hooks/use-mobile"
import { api, type AccessibleRepo, type GitHubAppInstallation } from "@/lib/api"
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
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { LabelCombobox } from "./LabelCombobox"
import type { WorkflowTriggerDraft } from "./workflow-draft"

export interface TriggerCardProps {
  tenantId: string
  installations: GitHubAppInstallation[]
  value: WorkflowTriggerDraft
  onChange: (next: WorkflowTriggerDraft) => void
}

export function TriggerCard({
  tenantId,
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
            <div className="min-w-0">
              <div className="text-xs uppercase tracking-wide text-muted-foreground">
                Trigger
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
          className={isMobile ? "rounded-t-xl" : ""}
        >
          <SheetHeader>
            <SheetTitle>Edit trigger</SheetTitle>
            <SheetDescription>
              Pick the install, repo, and label that starts the workflow.
            </SheetDescription>
          </SheetHeader>
          <div className="flex flex-1 flex-col gap-4 overflow-y-auto px-4">
            <TriggerForm
              tenantId={tenantId}
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
  tenantId,
  installations,
  value,
  onChange,
}: {
  tenantId: string
  installations: GitHubAppInstallation[]
  value: WorkflowTriggerDraft
  onChange: (next: WorkflowTriggerDraft) => void
}) {
  const installItems = installations.map((i) => ({
    value: i.id,
    label: `${i.account_login} (${i.account_type})`,
  }))
  const { data: reposData } = useQuery({
    queryKey: ["installation-repos", tenantId, value.install_id],
    queryFn: () =>
      value.install_id
        ? api.listInstallationRepos(tenantId, value.install_id)
        : Promise.resolve({ repos: [] as AccessibleRepo[] }),
    enabled: !!tenantId && !!value.install_id,
    staleTime: 30_000,
  })
  const repos = reposData?.repos ?? []
  const repoItems = repos.map((r) => ({
    value: `${r.owner}/${r.name}`,
    label: `${r.owner}/${r.name}`,
  }))
  const repoValue =
    value.repo_owner && value.repo_name
      ? `${value.repo_owner}/${value.repo_name}`
      : ""

  return (
    <>
      <div className="space-y-1.5">
        <span className="text-sm font-medium">GitHub install</span>
        <Select
          value={value.install_id}
          onValueChange={(v) =>
            onChange({
              ...value,
              install_id: v ?? "",
              repo_owner: "",
              repo_name: "",
              label: "",
            })
          }
          items={installItems}
          disabled={installations.length === 0}
        >
          <SelectTrigger className="w-full">
            <SelectValue placeholder={installations.length ? "Pick an install" : "No GitHub App installs yet"} />
          </SelectTrigger>
          <SelectContent>
            {installItems.map((i) => (
              <SelectItem key={i.value} value={i.value}>
                {i.label}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </div>

      <div className="space-y-1.5">
        <span className="text-sm font-medium">Repository</span>
        <Select
          value={repoValue}
          onValueChange={(v) => {
            const [owner, name] = (v ?? "").split("/")
            onChange({
              ...value,
              repo_owner: owner ?? "",
              repo_name: name ?? "",
              label: "",
            })
          }}
          items={repoItems}
          disabled={!value.install_id || repos.length === 0}
        >
          <SelectTrigger className="w-full">
            <SelectValue
              placeholder={value.install_id ? "Pick a repository" : "Pick an install first"}
            />
          </SelectTrigger>
          <SelectContent>
            {repoItems.map((i) => (
              <SelectItem key={i.value} value={i.value}>
                {i.label}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </div>

      <div className="space-y-1.5">
        <span className="text-sm font-medium">Trigger label</span>
        {value.install_id && value.repo_owner && value.repo_name ? (
          <LabelCombobox
            tenantId={tenantId}
            installId={value.install_id}
            repoOwner={value.repo_owner}
            repoName={value.repo_name}
            value={value.label || null}
            onChange={(label) => onChange({ ...value, label })}
          />
        ) : (
          <p className="flex items-center gap-2 text-xs text-muted-foreground">
            <GitBranch className="h-3.5 w-3.5" />
            Pick an install and repo above.
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
  if (!t.install_id) {
    return {
      title: "Pick an install",
      detail: "GitHub issue label will start the workflow",
    }
  }
  if (!t.repo_owner || !t.repo_name) {
    return { title: "Pick a repository", detail: "Install selected" }
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
