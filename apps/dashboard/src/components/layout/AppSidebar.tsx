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
import { useAuth } from "@/lib/auth"
import { CreateArtifactSheet } from "@/components/factory/CreateArtifactSheet"
import { Button } from "@/components/ui/button"
import { SetupChecklist } from "@/components/workspaces/SetupChecklist"
import { useSetupProgress } from "@/hooks/useSetupProgress"

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

// Sections kept visible before the user has created their first workspace.
// Everything else (factory, governance, infrastructure) shows empty tables in
// that state and distracts from the setup path, so we hide it until the user
// has at least one workspace.
const PRE_WORKSPACE_SECTIONS = new Set(["Organization", "System"])

export function AppSidebar() {
  const location = useLocation()
  const { user, authEnabled } = useAuth()
  const { isMobile, setOpenMobile } = useSidebar()
  // Call the progress hook once at the sidebar root and thread the result
  // down — avoids a second observer/render path when SetupChecklist mounts.
  const setupProgress = useSetupProgress()
  const { hasWorkspace, workspacesLoading } = setupProgress

  // The Organization section (workspaces) requires authentication; /api/tenants
  // returns 401 otherwise. Hide the group entirely in anonymous/L1 mode.
  const authed = authEnabled && !!user
  // Progressive disclosure: authenticated users with zero workspaces only see
  // the Organization + System groups. Anonymous users see everything except
  // Organization (same as before). Once the first workspace is created the
  // full nav unlocks — and the SetupChecklist takes over as outer-loop
  // guidance until GitHub is connected and a project is added. Gate on
  // workspacesLoading only (not the aggregate `loading`) so the nav decision
  // doesn't wait for the slower projects/installs queries.
  const gateNav = authed && !workspacesLoading && !hasWorkspace
  const visibleSections = navSections.filter((s) => {
    if (s.label === "Organization" && !authed) return false
    if (gateNav && !PRE_WORKSPACE_SECTIONS.has(s.label)) return false
    return true
  })

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
        <SetupChecklist progress={setupProgress} />
        {!gateNav && (
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
        )}
      </SidebarContent>
      <SidebarFooter className="border-t p-4">
        <span className="text-xs text-muted-foreground">v0.1.0</span>
      </SidebarFooter>
    </Sidebar>
  )
}
