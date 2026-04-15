import { LayoutDashboard, Plus, Server, Terminal, Settings, LogOut } from "lucide-react"
import { Link, useLocation } from "react-router-dom"
import {
  Sidebar,
  SidebarContent,
  SidebarFooter,
  SidebarGroup,
  SidebarGroupContent,
  SidebarGroupLabel,
  SidebarHeader,
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
} from "@/components/ui/sidebar"
import { ThemeToggle } from "./ThemeToggle"
import { useAuth } from "@/lib/auth"
import { CreateSessionSheet } from "@/components/sessions/CreateSessionSheet"
import { Button } from "@/components/ui/button"

const navItems = [
  { title: "Dashboard", icon: LayoutDashboard, path: "/" },
  { title: "Nodes", icon: Server, path: "/nodes" },
  { title: "Sessions", icon: Terminal, path: "/sessions" },
  { title: "Settings", icon: Settings, path: "/settings" },
]

export function AppSidebar() {
  const location = useLocation()
  const { user, authEnabled, logout } = useAuth()

  return (
    <Sidebar>
      <SidebarHeader className="border-b px-6 py-4">
        <Link to="/" className="flex items-center gap-2">
          <Terminal className="h-6 w-6 text-blue-500" />
          <span className="text-lg font-semibold">Stiglab</span>
        </Link>
      </SidebarHeader>
      <SidebarContent>
        <SidebarGroup>
          <SidebarGroupLabel>Navigation</SidebarGroupLabel>
          <SidebarGroupContent>
            <SidebarMenu>
              {navItems.map((item) => (
                <SidebarMenuItem key={item.title}>
                  <SidebarMenuButton
                    render={<Link to={item.path} />}
                    isActive={location.pathname === item.path}
                  >
                    <item.icon className="h-4 w-4" />
                    <span>{item.title}</span>
                  </SidebarMenuButton>
                </SidebarMenuItem>
              ))}
            </SidebarMenu>
          </SidebarGroupContent>
        </SidebarGroup>
        <SidebarGroup>
          <SidebarGroupContent>
            <CreateSessionSheet>
              <Button variant="outline" className="w-full justify-start gap-2">
                <Plus className="h-4 w-4" />
                <span>New Session</span>
              </Button>
            </CreateSessionSheet>
          </SidebarGroupContent>
        </SidebarGroup>
      </SidebarContent>
      <SidebarFooter className="border-t p-4 space-y-3">
        {authEnabled && user && (
          <div className="flex items-center gap-2">
            {user.github_avatar_url && (
              <img
                src={user.github_avatar_url}
                alt={user.github_login}
                className="h-6 w-6 rounded-full"
              />
            )}
            <span className="flex-1 truncate text-sm">
              {user.github_name ?? user.github_login}
            </span>
            <Button
              variant="ghost"
              size="sm"
              className="h-6 w-6 p-0"
              onClick={logout}
              title="Sign out"
            >
              <LogOut className="h-3.5 w-3.5" />
            </Button>
          </div>
        )}
        <div className="flex items-center justify-between">
          <span className="text-xs text-muted-foreground">v0.1.0</span>
          <ThemeToggle />
        </div>
      </SidebarFooter>
    </Sidebar>
  )
}
