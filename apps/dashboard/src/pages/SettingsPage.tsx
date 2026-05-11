import { Link } from "react-router-dom"
import { useAuth } from "@/lib/auth"
import { useHashTab } from "@/lib/useHashTab"
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card"
import { Button } from "@/components/ui/button"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { ArrowRight, KeyRound, User } from "lucide-react"
import { usePageHeader } from "@/components/layout/PageHeader"

const TABS = ["profile", "tokens"] as const
type TabValue = (typeof TABS)[number]

/**
 * Account-scoped settings (#305). Two tabs:
 * - Profile: the signed-in GitHub identity.
 * - Tokens: link out to the personal access token manager.
 *
 * Workspace-scoped settings live at `/workspaces/:slug/settings` —
 * reachable from the sidebar footer's avatar menu when a workspace is
 * active. Notifications are deferred to a follow-up spec.
 */
export function SettingsPage() {
  usePageHeader({ title: "Settings" })
  const { user } = useAuth()
  const [tab, setTab] = useHashTab<TabValue>(TABS, "profile")

  return (
    <div className="space-y-4 md:space-y-6">
      <div>
        <h1 className="hidden text-2xl font-bold tracking-tight md:block">
          Settings
        </h1>
        <p className="text-sm text-muted-foreground">
          Account profile and personal access tokens.
        </p>
      </div>

      <Tabs value={tab} onValueChange={setTab}>
        <TabsList>
          <TabsTrigger value="profile">Profile</TabsTrigger>
          <TabsTrigger value="tokens">Tokens</TabsTrigger>
        </TabsList>

        <TabsContent value="profile" className="space-y-4 md:space-y-6">
          {user && (
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
                    <p className="font-medium">
                      {user.github_name ?? user.github_login}
                    </p>
                    <p className="text-sm text-muted-foreground">
                      @{user.github_login}
                    </p>
                  </div>
                </div>
              </CardContent>
            </Card>
          )}
        </TabsContent>

        <TabsContent value="tokens" className="space-y-4 md:space-y-6">
          <Card>
            <CardHeader>
              <CardTitle className="flex items-center gap-2 text-base md:text-lg">
                <KeyRound className="h-4 w-4" />
                Personal access tokens
              </CardTitle>
              <CardDescription>
                Bearer tokens for calling the Onsager API from CLIs, agents,
                and scheduled jobs.
              </CardDescription>
            </CardHeader>
            <CardContent>
              <Button render={<Link to="/settings/tokens" />} variant="outline">
                Manage tokens
                <ArrowRight className="ml-1 h-4 w-4" />
              </Button>
            </CardContent>
          </Card>
        </TabsContent>
      </Tabs>
    </div>
  )
}
