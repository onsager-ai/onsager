import { useState } from "react"
import { Link } from "react-router-dom"
import { useQuery } from "@tanstack/react-query"
import { GitBranch, Plus } from "lucide-react"
import { api } from "@/lib/api"
import { useAuth } from "@/lib/auth"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card"
import { WorkflowBuilderSheet } from "@/components/factory/workflows/WorkflowBuilderSheet"
import { usePageHeader } from "@/components/layout/PageHeader"

export function WorkflowsPage() {
  usePageHeader({ title: "Workflows" })
  const { user, authEnabled } = useAuth()
  const authed = authEnabled ? !!user : true
  const [creating, setCreating] = useState(false)

  const { data, isLoading } = useQuery({
    queryKey: ["workflows", "user"],
    queryFn: () => api.listWorkflowsForUser(),
    enabled: authed,
  })
  const workflows = data?.workflows ?? []

  return (
    <div className="space-y-4 md:space-y-6">
      <div className="flex items-center justify-between gap-2">
        <div>
          <h1 className="hidden text-2xl font-bold tracking-tight md:block">Workflows</h1>
          <p className="text-sm text-muted-foreground">
            Triggers that drive artifacts through stages. Tap a workflow to view.
          </p>
        </div>
        <Button onClick={() => setCreating(true)} size="sm" className="shrink-0">
          <Plus className="h-4 w-4" />
          Create workflow
        </Button>
      </div>

      {isLoading ? (
        <p className="text-sm text-muted-foreground">Loading…</p>
      ) : workflows.length === 0 ? (
        <Card>
          <CardContent className="flex flex-col items-center gap-3 py-10 text-center">
            <div className="flex h-12 w-12 items-center justify-center rounded-full bg-primary/10 text-primary">
              <GitBranch className="h-6 w-6" />
            </div>
            <div>
              <p className="text-base font-medium">No workflows yet</p>
              <p className="text-sm text-muted-foreground">
                Set one up and the factory starts responding to GitHub events.
              </p>
            </div>
            <Button onClick={() => setCreating(true)} size="lg">
              <Plus className="h-4 w-4" />
              Create your first workflow
            </Button>
          </CardContent>
        </Card>
      ) : (
        <div className="space-y-3">
          {workflows.map((w) => (
            <Link
              key={w.id}
              to={`/workflows/${w.id}`}
              className="block focus:outline-none focus-visible:ring-2 focus-visible:ring-ring rounded-md"
            >
              <Card className="transition hover:border-primary/40">
                <CardHeader className="flex flex-row items-center justify-between gap-2 px-4 pb-2 pt-4">
                  <CardTitle className="truncate text-base">{w.name}</CardTitle>
                  <Badge variant={w.status === "active" ? "default" : "outline"}>
                    {w.status}
                  </Badge>
                </CardHeader>
                <CardContent className="space-y-1 px-4 pb-4 text-xs text-muted-foreground">
                  <div className="truncate">
                    Trigger: {w.trigger.repo_owner}/{w.trigger.repo_name}
                    {w.trigger.label ? ` · ${w.trigger.label}` : ""}
                  </div>
                </CardContent>
              </Card>
            </Link>
          ))}
        </div>
      )}

      <WorkflowBuilderSheet open={creating} onOpenChange={setCreating} />
    </div>
  )
}
