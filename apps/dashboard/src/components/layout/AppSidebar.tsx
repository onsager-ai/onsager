import { Building2, Factory, Plus, Server, Terminal, Settings, Shield, Package, Activity } from "lucide-react"
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
  useSidebar,
} from "@/components/ui/sidebar"
import { ThemeToggle } from "./ThemeToggle"
import { useAuth } from "@/lib/auth"
import { CreateArtifactSheet } from "@/components/factory/CreateArtifactSheet"
import { Button } from "@/components/ui/button"

const navSections = [
  {
    label: "Organization",
    items: [
      { title: "Workspaces", icon: Building2, path: "/workspaces" },
    ],
  },
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
  const { user, authEnabled } = useAuth()
  const { isMobile, setOpenMobile } = useSidebar()

  // The Organization section (workspaces) requires authentication; /api/tenants
  // returns 401 otherwise. Hide the group entirely in anonymous/L1 mode.
  const authed = authEnabled && !!user
  const visibleSections = navSections.filter(
    (s) => s.label !== "Organization" || authed,
  )

  const closeMobile = () => {
    if (isMobile) setOpenMobile(false)
  }

  return (
    <Sidebar>
      <SidebarHeader className="border-b px-6 py-4">
        <Link to="/" className="flex items-center gap-2" onClick={closeMobile}>
          <OnsagerLogo size={24} />
          <span className="text-lg font-semibold">Onsager</span>
        </Link>
      </SidebarHeader>
      <SidebarContent>
        {visibleSections.map((section) => (
          <SidebarGroup key={section.label}>
            <SidebarGroupLabel>{section.label}</SidebarGroupLabel>
            <SidebarGroupContent>
              <SidebarMenu>
                {section.items.map((item) => (
                  <SidebarMenuItem key={item.title}>
                    <SidebarMenuButton
                      render={<Link to={item.path} onClick={closeMobile} />}
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
            {user.github_avatar_url ? (
              <img
                src={user.github_avatar_url}
                alt={user.github_login}
                className="h-7 w-7 rounded-full"
              />
            ) : (
              <div className="flex h-7 w-7 items-center justify-center rounded-full bg-muted text-xs font-medium uppercase">
                {user.github_name?.[0] ?? user.github_login[0]}
              </div>
            )}
            <div className="min-w-0 flex-1">
              <p className="truncate text-sm font-medium">
                {user.github_name ?? user.github_login}
              </p>
              {user.github_name && (
                <p className="truncate text-xs text-muted-foreground">
                  @{user.github_login}
                </p>
              )}
            </div>
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
