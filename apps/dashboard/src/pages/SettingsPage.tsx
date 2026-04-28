import { Link } from "react-router-dom"
import { useAuth } from "@/lib/auth"
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/card"
import { Button } from "@/components/ui/button"
import { ArrowRight, Building2, KeyRound, User } from "lucide-react"
import { usePageHeader } from "@/components/layout/PageHeader"

/**
 * Account-wide settings (#166 settings split). Workspace-scoped settings
 * (credentials, projects, GitHub installs) live at
 * `/workspaces/:workspace/settings` since #166. This page covers only
 * the account itself: GitHub profile and personal access tokens.
 */
export function SettingsPage() {
  usePageHeader({ title: "Settings" })
  const { user, authEnabled } = useAuth()

  return (
    <div className="space-y-4 md:space-y-6">
      <div>
        <h1 className="hidden text-2xl font-bold tracking-tight md:block">Settings</h1>
        <p className="text-sm text-muted-foreground">
          Account profile and personal access tokens.
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

      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-2 text-base md:text-lg">
            <Building2 className="h-4 w-4" />
            Workspaces
          </CardTitle>
          <CardDescription>
            Manage GitHub installations, projects, members, and credentials
            per workspace.
          </CardDescription>
        </CardHeader>
        <CardContent>
          <Button render={<Link to="/workspaces" />} variant="outline">
            Go to Workspaces
            <ArrowRight className="ml-1 h-4 w-4" />
          </Button>
        </CardContent>
      </Card>

      {/* Personal Access Tokens link-out. Hidden in anonymous mode (no
          user to mint a token against). */}
      {authEnabled && user && (
        <Card>
          <CardHeader>
            <CardTitle className="flex items-center gap-2 text-base md:text-lg">
              <KeyRound className="h-4 w-4" />
              Personal access tokens
            </CardTitle>
            <CardDescription>
              Bearer tokens for calling the Onsager API from CLIs, agents, and
              scheduled jobs.
            </CardDescription>
          </CardHeader>
          <CardContent>
            <Button render={<Link to="/settings/tokens" />} variant="outline">
              Manage tokens
              <ArrowRight className="ml-1 h-4 w-4" />
            </Button>
          </CardContent>
        </Card>
      )}
    </div>
  )
}
