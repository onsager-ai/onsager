import { Link } from "react-router-dom"
import { useActiveWorkspace } from "@/lib/workspace"
import {
  Card,
  CardContent,
  CardHeader,
  CardTitle,
  CardDescription,
} from "@/components/ui/card"
import { Button } from "@/components/ui/button"
import { ArrowRight, Building2 } from "lucide-react"
import { usePageHeader } from "@/components/layout/PageHeader"
import { WorkspaceCredentials } from "@/components/workspaces/WorkspaceCredentials"
import { PushNotificationsCard } from "@/components/chat/PushNotificationsCard"

/**
 * Workspace-scoped settings (#305): GitHub install link-out,
 * credentials, and push notifications. The old Infrastructure tab
 * (registered agent nodes) retired with the agent-node fleet — sessions
 * run in-process in portal per ADR 0027 / spec #583.
 */
export function WorkspaceSettingsPage() {
  const workspace = useActiveWorkspace()
  usePageHeader({ title: `${workspace.name} · Settings` })

  return (
    <div className="space-y-4 md:space-y-6">
      <div>
        <h1 className="hidden text-2xl font-bold tracking-tight md:block">
          Workspace settings
        </h1>
        <p className="text-sm text-muted-foreground">
          Settings scoped to <strong>{workspace.name}</strong>.
        </p>
      </div>

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

      <WorkspaceCredentials workspace={workspace} />

      <PushNotificationsCard workspace={workspace} />
    </div>
  )
}
