import { Link, NavLink } from 'react-router-dom'
import { LogOut } from 'lucide-react'
import { OnsagerLogo } from '@/components/layout/OnsagerLogo'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { useAuth } from '@/lib/auth'

const NAV = [
  { to: '/workflows', label: 'Workflows' },
  { to: '/artifacts', label: 'Artifacts' },
]

/** The v2 app frame: top bar + nav. */
export function Shell({ children }: { children: React.ReactNode }) {
  const { user, sessionKind, logout } = useAuth()
  return (
    <div className="min-h-screen bg-background">
      <header className="flex items-center justify-between border-b px-6 py-3">
        <div className="flex items-center gap-6">
          <Link to="/" className="flex items-center gap-2">
            <OnsagerLogo size={24} />
            <span className="font-semibold">Onsager</span>
          </Link>
          <nav className="flex items-center gap-4 text-sm">
            {NAV.map((item) => (
              <NavLink
                key={item.to}
                to={item.to}
                className={({ isActive }) =>
                  isActive
                    ? 'font-medium text-foreground'
                    : 'text-muted-foreground hover:text-foreground'
                }
              >
                {item.label}
              </NavLink>
            ))}
          </nav>
          {sessionKind === 'dev' && <Badge variant="outline">dev mode</Badge>}
        </div>
        <div className="flex items-center gap-3 text-sm text-muted-foreground">
          <span>{user?.github_login}</span>
          <Button variant="ghost" size="sm" onClick={logout} aria-label="Log out">
            <LogOut className="h-4 w-4" />
          </Button>
        </div>
      </header>
      <main className="mx-auto max-w-4xl space-y-4 p-6">{children}</main>
    </div>
  )
}
