import { useState } from "react"
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query"
import { useAuth } from "@/lib/auth"
import { api } from "@/lib/api"
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/card"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Trash2, Plus, KeyRound, User } from "lucide-react"
import { WorkspacesCard } from "@/components/settings/WorkspacesCard"

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

export function SettingsPage() {
  const { user, authEnabled } = useAuth()
  const queryClient = useQueryClient()
  const [newCredName, setNewCredName] = useState("")
  const [newCredValue, setNewCredValue] = useState("")
  const [editingCred, setEditingCred] = useState<string | null>(null)
  const [editValue, setEditValue] = useState("")
  const [saveError, setSaveError] = useState<{ form: string; message: string } | null>(null)

  const { data: credData } = useQuery({
    queryKey: ["credentials"],
    queryFn: api.getCredentials,
  })

  const setCred = useMutation({
    mutationFn: ({ name, value }: { name: string; value: string }) =>
      api.setCredential(name, value),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["credentials"] })
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
    mutationFn: (name: string) => api.deleteCredential(name),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["credentials"] })
    },
  })

  const credentials = credData?.credentials ?? []
  const existingNames = new Set(credentials.map((c) => c.name))

  return (
    <div className="space-y-4 md:space-y-6">
      <div>
        <h1 className="text-xl font-bold tracking-tight md:text-2xl">Settings</h1>
        <p className="text-sm text-muted-foreground">
          Manage your profile and credentials.
        </p>
      </div>

      {/* Profile — only show when auth is enabled (not anonymous) */}
      {authEnabled && user && (
        <Card>
          <CardHeader>
            <CardTitle className="flex items-center gap-2 text-base md:text-lg">
              <User className="h-4 w-4" />
              Profile
            </CardTitle>
          </CardHeader>
          <CardContent>
            <div className="flex items-center gap-4">
              {user.github_avatar_url && (
                <img
                  src={user.github_avatar_url}
                  alt={user.github_login}
                  className="h-12 w-12 rounded-full"
                />
              )}
              <div>
                <p className="font-medium">{user.github_name ?? user.github_login}</p>
                <p className="text-sm text-muted-foreground">@{user.github_login}</p>
              </div>
            </div>
          </CardContent>
        </Card>
      )}

      {/* Workspaces */}
      <WorkspacesCard />

      {/* Credentials */}
      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-2 text-base md:text-lg">
            <KeyRound className="h-4 w-4" />
            Credentials
          </CardTitle>
          <CardDescription>
            Credentials are encrypted and passed to agent sessions as environment variables.
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          {/* Existing credentials */}
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
