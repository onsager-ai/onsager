import { useState } from "react"
import { Link } from "react-router-dom"
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query"
import { api } from "@/lib/api"
import { useActiveWorkspace } from "@/lib/workspace"
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/card"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { ArrowRight, Building2, Trash2, Plus, KeyRound } from "lucide-react"
import { usePageHeader } from "@/components/layout/PageHeader"

const KNOWN_CREDENTIALS = [
  {
    name: "CLAUDE_CODE_OAUTH_TOKEN",
    description: "OAuth token for Claude Code CLI authentication",
  },
  {
    name: "ANTHROPIC_API_KEY",
    description: "Anthropic API key for direct API access",
  },
]

/**
 * Per-workspace settings page (#166). Owns credentials (#164 makes them
 * workspace-scoped on the wire) and links out to the workspace's GitHub
 * App installations and projects, which live on the existing
 * `/workspaces` picker page where the WorkspaceCard renders them.
 */
export function WorkspaceSettingsPage() {
  const workspace = useActiveWorkspace()
  usePageHeader({ title: `${workspace.name} · Settings` })
  const queryClient = useQueryClient()
  const [newCredName, setNewCredName] = useState("")
  const [newCredValue, setNewCredValue] = useState("")
  const [editingCred, setEditingCred] = useState<string | null>(null)
  const [editValue, setEditValue] = useState("")
  const [saveError, setSaveError] = useState<{ form: string; message: string } | null>(null)

  // React Query keys carry the workspace id so cache lines don't collide
  // across workspaces — switching the active workspace doesn't show
  // stale credentials from the previous one.
  const credKey = ["credentials", workspace.id]

  const { data: credData } = useQuery({
    queryKey: credKey,
    queryFn: () => api.getCredentials(workspace.id),
  })

  const setCred = useMutation({
    mutationFn: ({ name, value }: { name: string; value: string }) =>
      api.setCredential(workspace.id, name, value),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: credKey })
      setNewCredName("")
      setNewCredValue("")
      setEditingCred(null)
      setEditValue("")
      setSaveError(null)
    },
    onError: (error) => {
      const message = error instanceof Error ? error.message : "Failed to save"
      setSaveError({ form: editingCred ?? "custom", message })
    },
  })

  const deleteCred = useMutation({
    mutationFn: (name: string) => api.deleteCredential(workspace.id, name),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: credKey })
    },
  })

  const credentials = credData?.credentials ?? []
  const existingNames = new Set(credentials.map((c) => c.name))

  return (
    <div className="space-y-4 md:space-y-6">
      <div>
        <h1 className="hidden text-2xl font-bold tracking-tight md:block">
          Workspace settings
        </h1>
        <p className="text-sm text-muted-foreground">
          Credentials and integrations scoped to <strong>{workspace.name}</strong>.
        </p>
      </div>

      {/* Workspace card link-out: GitHub installations + projects + members
          live on the WorkspacesPage card today. Until that surface is
          factored apart, point the user there from the per-workspace
          settings page so the model is consistent ("everything for this
          workspace lives at /workspaces/:workspace/settings"). */}
      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-2 text-base md:text-lg">
            <Building2 className="h-4 w-4" />
            Installations, projects, members
          </CardTitle>
          <CardDescription>
            Manage the GitHub App installation, opt-in projects, and members
            for this workspace.
          </CardDescription>
        </CardHeader>
        <CardContent>
          <Button render={<Link to="/workspaces" />} variant="outline">
            Open Workspaces
            <ArrowRight className="ml-1 h-4 w-4" />
          </Button>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-2 text-base md:text-lg">
            <KeyRound className="h-4 w-4" />
            Credentials
          </CardTitle>
          <CardDescription>
            Encrypted at rest and passed to agent sessions launched from this
            workspace as environment variables.
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          {credentials.map((cred) => (
            <div
              key={cred.name}
              className="space-y-2 rounded-md border p-3"
            >
              <div className="flex items-center justify-between gap-2">
                <div className="min-w-0 flex-1">
                  <p className="truncate font-mono text-sm font-medium">{cred.name}</p>
                  <p className="text-xs text-muted-foreground">
                    Updated {new Date(cred.updated_at).toLocaleDateString()}
                  </p>
                </div>
                {editingCred !== cred.name && (
                  <div className="flex shrink-0 items-center gap-2">
                    <Button
                      size="sm"
                      variant="outline"
                      onClick={() => {
                        setEditingCred(cred.name)
                        setEditValue("")
                        setSaveError(null)
                      }}
                    >
                      Update
                    </Button>
                    <Button
                      size="sm"
                      variant="outline"
                      onClick={() => deleteCred.mutate(cred.name)}
                      disabled={deleteCred.isPending}
                      aria-label={`Delete ${cred.name}`}
                      title={`Delete ${cred.name}`}
                    >
                      <Trash2 className="h-3 w-3" />
                    </Button>
                  </div>
                )}
              </div>
              {editingCred === cred.name && (
                <form
                  onSubmit={(e) => {
                    e.preventDefault()
                    if (setCred.isPending) return
                    if (editValue) setCred.mutate({ name: cred.name, value: editValue })
                  }}
                  className="space-y-2"
                >
                  <div className="flex items-center gap-2">
                    <Input
                      type="password"
                      placeholder="New value"
                      value={editValue}
                      onChange={(e) => setEditValue(e.target.value)}
                      className="flex-1"
                    />
                    <Button
                      size="sm"
                      type="submit"
                      disabled={!editValue || setCred.isPending}
                    >
                      Save
                    </Button>
                    <Button
                      size="sm"
                      type="button"
                      variant="outline"
                      onClick={() => {
                        setEditingCred(null)
                        setEditValue("")
                        setSaveError(null)
                      }}
                    >
                      Cancel
                    </Button>
                  </div>
                  {saveError?.form === cred.name && (
                    <p className="text-xs text-destructive">{saveError.message}</p>
                  )}
                </form>
              )}
            </div>
          ))}

          {/* Quick-add known credentials */}
          {KNOWN_CREDENTIALS.filter((k) => !existingNames.has(k.name)).length >
            0 && (
            <div className="space-y-2">
              <p className="text-sm font-medium text-muted-foreground">
                Add credential
              </p>
              {KNOWN_CREDENTIALS.filter((k) => !existingNames.has(k.name)).map(
                (known) => (
                  <div
                    key={known.name}
                    className="space-y-2 rounded-md border border-dashed p-3"
                  >
                    <div className="flex items-center justify-between gap-2">
                      <div className="min-w-0 flex-1">
                        <p className="truncate font-mono text-sm">{known.name}</p>
                        <p className="text-xs text-muted-foreground">
                          {known.description}
                        </p>
                      </div>
                      {editingCred !== `new-${known.name}` && (
                        <Button
                          size="sm"
                          variant="outline"
                          className="shrink-0"
                          onClick={() => {
                            setEditingCred(`new-${known.name}`)
                            setEditValue("")
                            setSaveError(null)
                          }}
                        >
                          <Plus className="mr-1 h-3 w-3" />
                          Add
                        </Button>
                      )}
                    </div>
                    {editingCred === `new-${known.name}` && (
                      <form
                        onSubmit={(e) => {
                          e.preventDefault()
                          if (setCred.isPending) return
                          if (editValue) setCred.mutate({ name: known.name, value: editValue })
                        }}
                        className="space-y-2"
                      >
                        <div className="flex items-center gap-2">
                          <Input
                            type="password"
                            placeholder="Value"
                            value={editValue}
                            onChange={(e) => setEditValue(e.target.value)}
                            className="flex-1"
                          />
                          <Button
                            size="sm"
                            type="submit"
                            disabled={!editValue || setCred.isPending}
                          >
                            Save
                          </Button>
                          <Button
                            size="sm"
                            type="button"
                            variant="outline"
                            onClick={() => {
                              setEditingCred(null)
                              setEditValue("")
                              setSaveError(null)
                            }}
                          >
                            Cancel
                          </Button>
                        </div>
                        {saveError?.form === `new-${known.name}` && (
                          <p className="text-xs text-destructive">{saveError.message}</p>
                        )}
                      </form>
                    )}
                  </div>
                )
              )}
            </div>
          )}

          {/* Custom credential */}
          <form
            onSubmit={(e) => {
              e.preventDefault()
              if (setCred.isPending) return
              if (newCredName && newCredValue) setCred.mutate({ name: newCredName, value: newCredValue })
            }}
            className="space-y-2 border-t pt-4"
          >
            <p className="text-sm font-medium text-muted-foreground">
              Custom credential
            </p>
            <div className="grid grid-cols-1 gap-2 sm:grid-cols-[auto_1fr_auto]">
              <Input
                placeholder="ENV_VAR_NAME"
                value={newCredName}
                onChange={(e) => setNewCredName(e.target.value.toUpperCase())}
                className="sm:w-48"
              />
              <Input
                type="password"
                placeholder="Value"
                value={newCredValue}
                onChange={(e) => setNewCredValue(e.target.value)}
              />
              <Button
                size="sm"
                type="submit"
                disabled={!newCredName || !newCredValue || setCred.isPending}
              >
                <Plus className="mr-1 h-3 w-3" />
                Add
              </Button>
            </div>
            {saveError?.form === "custom" && (
              <p className="text-xs text-destructive">{saveError.message}</p>
            )}
          </form>
        </CardContent>
      </Card>
    </div>
  )
}
