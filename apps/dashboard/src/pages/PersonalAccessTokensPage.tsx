import { useMemo, useState } from "react"
import { Link } from "react-router-dom"
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query"
import {
  ArrowLeft,
  Check,
  Copy,
  KeyRound,
  Plus,
  ShieldAlert,
  Trash2,
} from "lucide-react"

import { useAuth } from "@/lib/auth"
import { api, type Pat } from "@/lib/api"
import { usePageHeader } from "@/components/layout/PageHeader"
import { Button } from "@/components/ui/button"
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { Input } from "@/components/ui/input"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"

// GitHub-style expiry choices. v1 spec (#143) freezes the option set so
// users can't accidentally mint a never-expiring token from the UI.
const EXPIRY_CHOICES = [
  { id: "7", label: "7 days", days: 7 },
  { id: "30", label: "30 days", days: 30 },
  { id: "60", label: "60 days", days: 60 },
  { id: "90", label: "90 days", days: 90 },
  { id: "custom", label: "Custom date" },
] as const

type ExpiryChoiceId = (typeof EXPIRY_CHOICES)[number]["id"]

function expiryFromChoice(
  choice: ExpiryChoiceId,
  customDate: string,
): { iso: string; error?: string } {
  if (choice === "custom") {
    if (!customDate) {
      return { iso: "", error: "Pick a custom date" }
    }
    const d = new Date(customDate)
    if (Number.isNaN(d.getTime()) || d.getTime() <= Date.now()) {
      return { iso: "", error: "Custom date must be in the future" }
    }
    return { iso: d.toISOString() }
  }
  const days = EXPIRY_CHOICES.find((c) => c.id === choice)
  if (!days || days.id === "custom") {
    return { iso: "", error: "Select an expiry" }
  }
  const d = new Date(Date.now() + days.days * 24 * 60 * 60 * 1000)
  return { iso: d.toISOString() }
}

function formatTimestamp(ts: string | null): string {
  if (!ts) return "—"
  return new Date(ts).toLocaleString()
}

// Truncate a UA string for table cells. Full string is available on the
// `title` attribute for hover.
function truncate(s: string | null, max = 40): string {
  if (!s) return "—"
  return s.length <= max ? s : `${s.slice(0, max - 1)}…`
}

interface CreatePatDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  workspaces: { id: string; name: string }[]
}

function CreatePatDialog({ open, onOpenChange, workspaces }: CreatePatDialogProps) {
  const queryClient = useQueryClient()
  const [name, setName] = useState("")
  const [tenantId, setTenantId] = useState<string>("__all__")
  const [expiryChoice, setExpiryChoice] = useState<ExpiryChoiceId>("30")
  const [customDate, setCustomDate] = useState("")
  const [revealedToken, setRevealedToken] = useState<string | null>(null)
  const [copied, setCopied] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const create = useMutation({
    mutationFn: (body: {
      name: string
      tenant_id: string | null
      expires_at: string
    }) =>
      api.createPat({
        name: body.name,
        tenant_id: body.tenant_id,
        expires_at: body.expires_at,
      }),
    onSuccess: (data) => {
      setRevealedToken(data.token)
      queryClient.invalidateQueries({ queryKey: ["pats"] })
    },
    onError: (e) => {
      setError(e instanceof Error ? e.message : "Failed to create token")
    },
  })

  function reset() {
    setName("")
    setTenantId("__all__")
    setExpiryChoice("30")
    setCustomDate("")
    setRevealedToken(null)
    setCopied(false)
    setError(null)
  }

  function handleClose(next: boolean) {
    if (!next) reset()
    onOpenChange(next)
  }

  function handleSubmit(e: React.FormEvent) {
    e.preventDefault()
    if (create.isPending) return
    setError(null)
    const trimmed = name.trim()
    if (!trimmed) {
      setError("Name is required")
      return
    }
    const exp = expiryFromChoice(expiryChoice, customDate)
    if (exp.error) {
      setError(exp.error)
      return
    }
    create.mutate({
      name: trimmed,
      tenant_id: tenantId === "__all__" ? null : tenantId,
      expires_at: exp.iso,
    })
  }

  async function copyToken() {
    if (!revealedToken) return
    try {
      await navigator.clipboard.writeText(revealedToken)
      setCopied(true)
      setTimeout(() => setCopied(false), 2000)
    } catch {
      // Some browsers (older Safari, denied permissions) reject the
      // clipboard write — fall back to letting the user select manually.
      setError("Copy failed; select the token text and copy manually")
    }
  }

  return (
    <Dialog open={open} onOpenChange={handleClose}>
      <DialogContent className="sm:max-w-md">
        {!revealedToken ? (
          <form onSubmit={handleSubmit}>
            <DialogHeader>
              <DialogTitle>New personal access token</DialogTitle>
              <DialogDescription>
                You won't be able to see the token again after closing this
                dialog. Copy it before navigating away.
              </DialogDescription>
            </DialogHeader>
            <div className="space-y-4 py-4">
              <div className="space-y-2">
                <label htmlFor="pat-name" className="text-sm font-medium">
                  Name
                </label>
                <Input
                  id="pat-name"
                  placeholder="e.g. ci, my-laptop"
                  value={name}
                  onChange={(e) => setName(e.target.value)}
                  autoFocus
                />
              </div>

              <div className="space-y-2">
                <label htmlFor="pat-workspace" className="text-sm font-medium">
                  Workspace
                </label>
                <Select
                  value={tenantId}
                  onValueChange={(v) => setTenantId(v ?? "__all__")}
                >
                  <SelectTrigger id="pat-workspace">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="__all__">All workspaces</SelectItem>
                    {workspaces.map((w) => (
                      <SelectItem key={w.id} value={w.id}>
                        {w.name}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
                <p className="text-xs text-muted-foreground">
                  Restricting to a workspace blocks the token from touching
                  other workspaces.
                </p>
              </div>

              <div className="space-y-2">
                <label htmlFor="pat-expiry" className="text-sm font-medium">
                  Expiration
                </label>
                <Select
                  value={expiryChoice}
                  onValueChange={(v) =>
                    v && setExpiryChoice(v as ExpiryChoiceId)
                  }
                >
                  <SelectTrigger id="pat-expiry">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {EXPIRY_CHOICES.map((c) => (
                      <SelectItem key={c.id} value={c.id}>
                        {c.label}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
                {expiryChoice === "custom" && (
                  <Input
                    type="date"
                    value={customDate}
                    onChange={(e) => setCustomDate(e.target.value)}
                  />
                )}
              </div>

              {error && <p className="text-sm text-destructive">{error}</p>}
            </div>
            <DialogFooter>
              <Button
                type="button"
                variant="outline"
                onClick={() => handleClose(false)}
              >
                Cancel
              </Button>
              <Button type="submit" disabled={create.isPending}>
                {create.isPending ? "Creating…" : "Create token"}
              </Button>
            </DialogFooter>
          </form>
        ) : (
          <>
            <DialogHeader>
              <DialogTitle>Token created</DialogTitle>
              <DialogDescription>
                Copy this token now. It won't be shown again.
              </DialogDescription>
            </DialogHeader>
            <div className="space-y-3 py-4">
              <div className="flex items-center gap-2">
                <Input
                  readOnly
                  value={revealedToken}
                  className="font-mono text-xs"
                  onFocus={(e) => e.currentTarget.select()}
                />
                <Button type="button" onClick={copyToken} variant="outline">
                  {copied ? (
                    <>
                      <Check className="mr-1 h-4 w-4" /> Copied
                    </>
                  ) : (
                    <>
                      <Copy className="mr-1 h-4 w-4" /> Copy
                    </>
                  )}
                </Button>
              </div>
              <p className="text-xs text-muted-foreground">
                Use it as <code>Authorization: Bearer {"<token>"}</code> on
                requests to the Onsager API.
              </p>
              {error && <p className="text-sm text-destructive">{error}</p>}
            </div>
            <DialogFooter>
              <Button type="button" onClick={() => handleClose(false)}>
                Done
              </Button>
            </DialogFooter>
          </>
        )}
      </DialogContent>
    </Dialog>
  )
}

interface PatRowProps {
  pat: Pat
  workspaceName: (id: string | null) => string
  onRevoke: (id: string) => void
  isRevoking: boolean
}

function PatRow({ pat, workspaceName, onRevoke, isRevoking }: PatRowProps) {
  const expired =
    pat.expires_at != null && new Date(pat.expires_at) <= new Date()
  return (
    <div className="space-y-2 rounded-md border p-3">
      <div className="flex flex-wrap items-start justify-between gap-2">
        <div className="min-w-0 flex-1 space-y-1">
          <div className="flex flex-wrap items-center gap-2">
            <p className="truncate font-medium">{pat.name}</p>
            <code className="rounded bg-muted px-1.5 py-0.5 font-mono text-xs">
              {pat.token_prefix}…
            </code>
            {pat.revoked_at && (
              <span className="rounded bg-muted px-1.5 py-0.5 text-xs">
                Revoked
              </span>
            )}
            {!pat.revoked_at && expired && (
              <span className="rounded bg-muted px-1.5 py-0.5 text-xs">
                Expired
              </span>
            )}
          </div>
          <p className="text-xs text-muted-foreground">
            Workspace: {workspaceName(pat.tenant_id)} · Expires{" "}
            {formatTimestamp(pat.expires_at)}
          </p>
          <p
            className="text-xs text-muted-foreground"
            title={pat.last_used_user_agent ?? undefined}
          >
            Last used: {formatTimestamp(pat.last_used_at)}
            {pat.last_used_ip ? ` from ${pat.last_used_ip}` : ""}
            {pat.last_used_user_agent
              ? ` (${truncate(pat.last_used_user_agent)})`
              : ""}
          </p>
        </div>
        {!pat.revoked_at && (
          <Button
            size="sm"
            variant="outline"
            onClick={() => onRevoke(pat.id)}
            disabled={isRevoking}
            aria-label={`Revoke ${pat.name}`}
            title={`Revoke ${pat.name}`}
          >
            <Trash2 className="h-3 w-3" />
          </Button>
        )}
      </div>
    </div>
  )
}

export function PersonalAccessTokensPage() {
  usePageHeader({ title: "Tokens", backTo: "/settings" })
  const { user, authEnabled } = useAuth()
  const queryClient = useQueryClient()
  const [createOpen, setCreateOpen] = useState(false)

  const patsQuery = useQuery({
    queryKey: ["pats"],
    queryFn: api.listPats,
    // PATs only make sense for an authenticated user — anonymous mode
    // renders the "unavailable" stub below, so don't fire a doomed
    // request that would just 401 and clutter logs.
    enabled: authEnabled && !!user,
  })
  const workspacesQuery = useQuery({
    queryKey: ["workspaces"],
    queryFn: api.listWorkspaces,
    enabled: authEnabled && !!user,
  })

  const revoke = useMutation({
    mutationFn: (id: string) => api.revokePat(id),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ["pats"] }),
  })

  const workspaces = useMemo(
    () => workspacesQuery.data?.tenants ?? [],
    [workspacesQuery.data],
  )
  const workspaceName = useMemo(() => {
    const byId = new Map(workspaces.map((w) => [w.id, w.name]))
    return (id: string | null) =>
      id == null ? "All workspaces" : byId.get(id) ?? id
  }, [workspaces])

  // Anonymous mode (auth disabled or no session) has no concept of a user
  // to mint a PAT against — render a stub instead of a 401.
  if (authEnabled && !user) {
    return null
  }
  if (!authEnabled) {
    return (
      <div className="space-y-4">
        <Card>
          <CardHeader>
            <CardTitle className="flex items-center gap-2">
              <ShieldAlert className="h-4 w-4" />
              Tokens unavailable
            </CardTitle>
            <CardDescription>
              Personal access tokens require authentication. Configure GitHub
              OAuth on this server to enable them.
            </CardDescription>
          </CardHeader>
        </Card>
      </div>
    )
  }

  const pats = patsQuery.data?.pats ?? []

  return (
    <div className="space-y-4 md:space-y-6">
      <div className="flex flex-wrap items-start justify-between gap-2">
        <div>
          <Button
            render={<Link to="/settings" />}
            variant="ghost"
            size="sm"
            className="-ml-2 mb-1 hidden md:inline-flex"
          >
            <ArrowLeft className="mr-1 h-4 w-4" />
            Settings
          </Button>
          <h1 className="hidden text-2xl font-bold tracking-tight md:block">
            Personal access tokens
          </h1>
          <p className="text-sm text-muted-foreground">
            Bearer tokens for calling the Onsager API from CLIs, agents, and
            scheduled jobs.
          </p>
        </div>
        <Button onClick={() => setCreateOpen(true)} className="hidden md:inline-flex">
          <Plus className="mr-1 h-4 w-4" />
          Create token
        </Button>
      </div>

      {pats.length === 0 ? (
        <Card>
          <CardHeader>
            <CardTitle className="flex items-center gap-2 text-base md:text-lg">
              <KeyRound className="h-4 w-4" />
              No tokens yet
            </CardTitle>
            <CardDescription>
              Create a token to call <code>/api</code> from outside the
              dashboard. Pass it as <code>Authorization: Bearer &lt;token&gt;</code>.
            </CardDescription>
          </CardHeader>
          <CardContent>
            <Button onClick={() => setCreateOpen(true)}>
              <Plus className="mr-1 h-4 w-4" />
              Create token
            </Button>
          </CardContent>
        </Card>
      ) : (
        <Card>
          <CardHeader>
            <CardTitle className="flex items-center gap-2 text-base md:text-lg">
              <KeyRound className="h-4 w-4" />
              Active tokens
            </CardTitle>
            <CardDescription>
              Revoking a token immediately invalidates any future request that
              presents it.
            </CardDescription>
          </CardHeader>
          <CardContent className="space-y-3">
            {pats.map((pat) => (
              <PatRow
                key={pat.id}
                pat={pat}
                workspaceName={workspaceName}
                onRevoke={(id) => revoke.mutate(id)}
                isRevoking={revoke.isPending}
              />
            ))}
            <Button
              variant="outline"
              size="sm"
              className="md:hidden"
              onClick={() => setCreateOpen(true)}
            >
              <Plus className="mr-1 h-4 w-4" />
              Create token
            </Button>
          </CardContent>
        </Card>
      )}

      <CreatePatDialog
        open={createOpen}
        onOpenChange={setCreateOpen}
        workspaces={workspaces.map((w) => ({ id: w.id, name: w.name }))}
      />
    </div>
  )
}
