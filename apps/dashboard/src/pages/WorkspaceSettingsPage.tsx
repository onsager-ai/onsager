import { Link } from "react-router-dom"
import { useActiveWorkspace } from "@/lib/workspace"
import { useHashTab } from "@/lib/useHashTab"
import { useNodes } from "@/hooks/useNodes"
import {
  Card,
  CardContent,
  CardHeader,
  CardTitle,
  CardDescription,
} from "@/components/ui/card"
import { Button } from "@/components/ui/button"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { ArrowRight, Building2 } from "lucide-react"
import { usePageHeader } from "@/components/layout/PageHeader"
import { WorkspaceCredentials } from "@/components/workspaces/WorkspaceCredentials"
import { NodeTable } from "@/components/nodes/NodeTable"
import { GovernanceEventsList } from "@/components/governance/GovernanceEventsList"

const TABS = ["workspace", "infrastructure", "governance"] as const
type TabValue = (typeof TABS)[number]

/**
 * Workspace-scoped settings (#305). Three tabs:
 * - Workspace: GitHub install link-out + credentials.
 * - Infrastructure: registered agent nodes (folds the old /nodes page).
 * - Governance audit: events list scoped to this workspace.
 *
 * Tab selection persists in the URL hash so deep links and reloads
 * preserve which tab the user was on.
 */
export function WorkspaceSettingsPage() {
  const workspace = useActiveWorkspace()
  usePageHeader({ title: `${workspace.name} · Settings` })
  const [tab, setTab] = useHashTab<TabValue>(TABS, "workspace")

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

      <Tabs value={tab} onValueChange={setTab}>
        <TabsList>
          <TabsTrigger value="workspace">Workspace</TabsTrigger>
          <TabsTrigger value="infrastructure">Infrastructure</TabsTrigger>
          <TabsTrigger value="governance">Governance audit</TabsTrigger>
        </TabsList>

        <TabsContent value="workspace" className="space-y-4 md:space-y-6">
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
        </TabsContent>

        <TabsContent value="infrastructure" className="space-y-4 md:space-y-6">
          <InfrastructureTab />
        </TabsContent>

        <TabsContent value="governance" className="space-y-4 md:space-y-6">
          <GovernanceEventsList workspaceId={workspace.id} />
        </TabsContent>
      </Tabs>
    </div>
  )
}

function InfrastructureTab() {
  const { data, isLoading, isError, error } = useNodes()
  const nodes = data?.nodes ?? []

  return (
    <Card>
      <CardHeader className="px-4 md:px-6">
        <CardTitle className="text-base md:text-lg">Registered nodes</CardTitle>
        <CardDescription>
          Agent nodes available to run sessions for this workspace.
        </CardDescription>
      </CardHeader>
      <CardContent className="px-4 md:px-6">
        {isLoading ? (
          <p className="py-8 text-center text-muted-foreground">Loading...</p>
        ) : isError ? (
          <p className="py-8 text-center text-sm text-destructive">
            Failed to load nodes
            {error instanceof Error ? `: ${error.message}` : "."}
          </p>
        ) : (
          <NodeTable nodes={nodes} />
        )}
      </CardContent>
    </Card>
  )
}
