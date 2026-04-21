import { useMemo, useState } from "react"
import { Link, useNavigate, useSearchParams } from "react-router-dom"
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query"
import { Factory, Loader2, Zap } from "lucide-react"
import { api, type AccessibleRepo, type CreateWorkflowRequest } from "@/lib/api"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card"
import { LabelCombobox } from "@/components/factory/workflows/LabelCombobox"
import {
  GITHUB_ISSUE_TO_PR_PRESET,
  draftToRequestTrigger,
  githubIssueToPrPreset,
} from "@/components/factory/workflows/workflow-draft"

/**
 * The 60-second "start the factory" card shown after a GitHub App install.
 * Lists each repo accessible to the install, with a label combobox + a
 * "Run factory" toggle per row. Activating a row creates a workflow from
 * the `github-issue-to-pr` preset and marks it active.
 */
export function WorkflowStartPage() {
  const [params] = useSearchParams()
  const installId = params.get("install") ?? ""
  const tenantIdParam = params.get("tenant_id") ?? ""

  const { data: workspacesData } = useQuery({
    queryKey: ["workspaces"],
    queryFn: api.listWorkspaces,
    staleTime: 30_000,
  })
  const workspaces = useMemo(
    () => workspacesData?.tenants ?? [],
    [workspacesData],
  )

  // If the callback URL doesn't carry an explicit tenant_id, fall back to
  // the workspace that owns this install — one query hop, no user typing.
  const { data: tenantInstallsData } = useQuery({
    queryKey: ["workflow-start-installations", workspaces.map((w) => w.id).join(",")],
    queryFn: async () => {
      const entries = await Promise.all(
        workspaces.map(async (w) => {
          const r = await api.listWorkspaceInstallations(w.id)
          return { tenantId: w.id, installations: r.installations }
        }),
      )
      return entries
    },
    enabled: workspaces.length > 0,
    staleTime: 30_000,
  })

  // Resolve the tenant that owns the given install id. If we can't match,
  // return "" and let the UI render an error — guessing a tenant would
  // run later mutations against the wrong workspace.
  const resolvedTenantId = useMemo(() => {
    if (tenantIdParam) return tenantIdParam
    if (!tenantInstallsData) return ""
    const hit = tenantInstallsData.find((e) =>
      e.installations.some((i) => i.id === installId),
    )
    return hit?.tenantId ?? ""
  }, [tenantIdParam, tenantInstallsData, installId])

  const installLookupDone = !!tenantIdParam || !!tenantInstallsData
  const installUnresolved =
    !!installId && installLookupDone && !resolvedTenantId

  const { data: reposData, isLoading: reposLoading } = useQuery({
    queryKey: ["installation-repos", resolvedTenantId, installId],
    queryFn: () => api.listInstallationRepos(resolvedTenantId, installId),
    enabled: !!resolvedTenantId && !!installId,
    retry: false,
  })
  const repos = reposData?.repos ?? []

  return (
    <div className="mx-auto max-w-2xl space-y-4 md:space-y-6">
      <div className="flex items-center gap-3">
        <div className="flex h-10 w-10 items-center justify-center rounded-full bg-primary/10 text-primary">
          <Factory className="h-5 w-5" />
        </div>
        <div>
          <h1 className="text-xl font-bold tracking-tight md:text-2xl">
            Start the factory
          </h1>
          <p className="text-sm text-muted-foreground">
            Pick a repo, tag a label, turn it on. You&apos;re done in a minute.
          </p>
        </div>
      </div>

      <Card>
        <CardHeader className="px-4 pb-2 pt-4 md:px-6">
          <CardTitle className="text-base">Connect a repo</CardTitle>
        </CardHeader>
        <CardContent className="space-y-3 px-4 pb-4 md:px-6">
          {!installId ? (
            <EmptyInstall />
          ) : installUnresolved ? (
            <p className="text-sm text-destructive">
              Couldn&apos;t find the workspace that owns this install. Try
              re-running the install flow from Workspaces.
            </p>
          ) : reposLoading ? (
            <p className="flex items-center gap-2 text-sm text-muted-foreground">
              <Loader2 className="h-4 w-4 animate-spin" />
              Loading repositories…
            </p>
          ) : repos.length === 0 ? (
            <p className="text-sm text-muted-foreground">
              The install doesn&apos;t have any accessible repositories yet.
            </p>
          ) : (
            <ul className="space-y-3">
              {repos.map((r) => (
                <RepoRow
                  key={`${r.owner}/${r.name}`}
                  repo={r}
                  tenantId={resolvedTenantId}
                  installId={installId}
                />
              ))}
            </ul>
          )}
        </CardContent>
      </Card>

      <p className="text-center text-xs text-muted-foreground">
        Want something custom?{" "}
        <Link to="/workflows" className="underline">
          Build a workflow from scratch
        </Link>
        .
      </p>
    </div>
  )
}

function EmptyInstall() {
  return (
    <div className="space-y-2">
      <p className="text-sm text-muted-foreground">
        No GitHub App install in this link. Connect one from Workspaces to
        continue.
      </p>
      <Button variant="outline" render={<Link to="/workspaces" />}>
        Go to Workspaces
      </Button>
    </div>
  )
}

function RepoRow({
  repo,
  tenantId,
  installId,
}: {
  repo: AccessibleRepo
  tenantId: string
  installId: string
}) {
  const [label, setLabel] = useState<string | null>(null)
  const queryClient = useQueryClient()
  const navigate = useNavigate()

  const create = useMutation({
    mutationFn: (body: CreateWorkflowRequest) => api.createWorkflow(body),
    onSuccess: ({ workflow }) => {
      queryClient.invalidateQueries({ queryKey: ["workflows"] })
      navigate(`/workflows/${workflow.id}`)
    },
  })

  const ready = !!label && !!tenantId
  const run = () => {
    if (!ready || !label) return
    const draft = githubIssueToPrPreset({
      install_id: installId,
      repo_owner: repo.owner,
      repo_name: repo.name,
      label,
    })
    create.mutate({
      tenant_id: tenantId,
      name: draft.name,
      preset: GITHUB_ISSUE_TO_PR_PRESET,
      trigger: draftToRequestTrigger(draft.trigger),
      stages: draft.stages,
      activate: true,
    })
  }

  return (
    <li className="rounded-lg border p-3">
      <div className="mb-2 flex items-center justify-between gap-2">
        <div className="min-w-0">
          <div className="truncate text-sm font-medium">
            {repo.owner}/{repo.name}
          </div>
          {repo.default_branch && (
            <div className="truncate text-xs text-muted-foreground">
              default: {repo.default_branch}
            </div>
          )}
        </div>
        {repo.private && <Badge variant="outline">private</Badge>}
      </div>
      <div className="space-y-2">
        <LabelCombobox
          tenantId={tenantId}
          installId={installId}
          repoOwner={repo.owner}
          repoName={repo.name}
          value={label}
          onChange={setLabel}
          placeholder="Pick a trigger label"
        />
        <Button
          type="button"
          size="lg"
          className="w-full"
          disabled={!ready || create.isPending}
          onClick={run}
        >
          <Zap className="h-4 w-4" />
          {create.isPending ? "Starting…" : "Run factory"}
        </Button>
        {create.isError && (
          <p className="text-xs text-destructive">
            {create.error instanceof Error
              ? create.error.message
              : "Failed to start"}
          </p>
        )}
      </div>
    </li>
  )
}
