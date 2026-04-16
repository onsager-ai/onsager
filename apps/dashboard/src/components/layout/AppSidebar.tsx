import { Factory, Plus, Server, Terminal, Settings, LogOut, Shield, Package, Activity } from "lucide-react"
import { OnsagerLogo } from "./OnsagerLogo"
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
import { CreateArtifactSheet } from "@/components/factory/CreateArtifactSheet"
import { Button } from "@/components/ui/button"

const navSections = [
  {
    label: "Factory",
    items: [
      { title: "Overview", icon: Factory, path: "/" },
      { title: "Artifacts", icon: Package, path: "/artifacts" },
      { title: "Event Spine", icon: Activity, path: "/spine" },
    ],
  },
  {
    label: "Governance",
    items: [
      { title: "Governance", icon: Shield, path: "/governance" },
    ],
  },
  {
    label: "Infrastructure",
    items: [
      { title: "Sessions", icon: Terminal, path: "/sessions" },
      { title: "Nodes", icon: Server, path: "/nodes" },
    ],
  },
  {
    label: "System",
    items: [
      { title: "Settings", icon: Settings, path: "/settings" },
    ],
  },
]

export function AppSidebar() {
  const location = useLocation()
  const { user, authEnabled, logout } = useAuth()

  return (
    <Sidebar>
      <SidebarHeader className="border-b px-6 py-4">
        <Link to="/" className="flex items-center gap-2">
          <OnsagerLogo size={24} />
          <span className="text-lg font-semibold">Onsager</span>
        </Link>
      </SidebarHeader>
      <SidebarContent>
        {navSections.map((section) => (
          <SidebarGroup key={section.label}>
            <SidebarGroupLabel>{section.label}</SidebarGroupLabel>
            <SidebarGroupContent>
              <SidebarMenu>
                {section.items.map((item) => (
                  <SidebarMenuItem key={item.title}>
                    <SidebarMenuButton
                      render={<Link to={item.path} />}
                      isActive={
                        item.path === "/"
                          ? location.pathname === "/"
                          : location.pathname.startsWith(item.path)
                      }
                    >
                      <item.icon className="h-4 w-4" />
                      <span>{item.title}</span>
                    </SidebarMenuButton>
                  </SidebarMenuItem>
                ))}
              </SidebarMenu>
            </SidebarGroupContent>
          </SidebarGroup>
        ))}
        <SidebarGroup>
          <SidebarGroupContent>
            <CreateArtifactSheet>
              <Button variant="outline" className="w-full justify-start gap-2">
                <Plus className="h-4 w-4" />
                <span>Register Artifact</span>
              </Button>
            </CreateArtifactSheet>
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
