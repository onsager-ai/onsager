import { useQuery } from '@tanstack/react-query'
import { LogOut } from 'lucide-react'
import { OnsagerLogo } from '@/components/layout/OnsagerLogo'
import { Button } from '@/components/ui/button'
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from '@/components/ui/card'
import { Badge } from '@/components/ui/badge'
import { api } from '@/lib/api'
import { useAuth } from '@/lib/auth'

/**
 * M0 home — proves the v2 shell end-to-end: session cookie → /api/me →
 * workspace list. The real navigation (Workflows / Runs / Artifacts)
 * arrives with its pages in M1.
 */
export function HomePage() {
  const { user, sessionKind, logout } = useAuth()
  const { data, isLoading } = useQuery({
    queryKey: ['workspaces'],
    queryFn: api.listWorkspaces,
  })

  return (
    <div className="min-h-screen bg-background">
      <header className="flex items-center justify-between border-b px-6 py-3">
        <div className="flex items-center gap-2">
          <OnsagerLogo size={24} />
          <span className="font-semibold">Onsager</span>
          {sessionKind === 'dev' && <Badge variant="outline">dev mode</Badge>}
        </div>
        <div className="flex items-center gap-3 text-sm text-muted-foreground">
          <span>{user?.github_login}</span>
          <Button variant="ghost" size="sm" onClick={logout} aria-label="Log out">
            <LogOut className="h-4 w-4" />
          </Button>
        </div>
      </header>

      <main className="mx-auto max-w-2xl space-y-4 p-6">
        <h1 className="text-xl font-bold tracking-tight">Workspaces</h1>
        {isLoading && <p className="text-sm text-muted-foreground">Loading...</p>}
        {data?.workspaces.map((ws) => (
          <Card key={ws.id}>
            <CardHeader>
              <CardTitle>{ws.name}</CardTitle>
              <CardDescription className="font-mono text-xs">{ws.slug}</CardDescription>
            </CardHeader>
            <CardContent className="text-sm text-muted-foreground">
              Workflows, runs, and artifacts arrive with M1.
            </CardContent>
          </Card>
        ))}
        {data && data.workspaces.length === 0 && (
          <p className="text-sm text-muted-foreground">No workspaces yet.</p>
        )}
      </main>
    </div>
  )
}
