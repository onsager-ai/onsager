import { Factory, GitBranch, Inbox, Server, Terminal, Settings, Shield, Package, Activity } from "lucide-react"
import { OnsagerLogo } from "./OnsagerLogo"
import { UserMenu } from "./UserMenu"
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
import { useOptionalActiveWorkspace } from "@/lib/workspace"
import { SetupChecklist } from "@/components/workspaces/SetupChecklist"
import { useSetupProgress } from "@/hooks/useSetupProgress"
import { WorkspaceSwitcher } from "@/components/workspaces/WorkspaceSwitcher"

interface NavItem {
  title: string
  icon: typeof Factory
  /** Path relative to the active workspace root (`/workspaces/<slug>/...`). */
  path: string
}

interface NavSection {
  label: string
  items: NavItem[]
}

// Resource sections live under the active workspace. The path here is
// the suffix appended to `/workspaces/<slug>/`. The Overview row uses
// an empty suffix to mean "the workspace root".
const SCOPED_SECTIONS: NavSection[] = [
  {
    label: "Factory",
    items: [
      { title: "Overview", icon: Factory, path: "" },
      // Issues inbox (#168) — reference-only `Kind::GithubIssue` artifacts
      // hydrated live via the portal proxy.
      { title: "Issues", icon: Inbox, path: "issues" },
      { title: "Workflows", icon: GitBranch, path: "workflows" },
      { title: "Artifacts", icon: Package, path: "artifacts" },
      { title: "Event Spine", icon: Activity, path: "spine" },
    ],
  },
  {
    label: "Governance",
    items: [
      { title: "Governance", icon: Shield, path: "governance" },
    ],
  },
  {
    label: "Infrastructure",
    items: [
      { title: "Sessions", icon: Terminal, path: "sessions" },
      { title: "Nodes", icon: Server, path: "nodes" },
    ],
  },
  {
    label: "System",
    items: [
      { title: "Settings", icon: Settings, path: "settings" },
    ],
  },
]

export function AppSidebar() {
  const location = useLocation()
  const { user, authEnabled } = useAuth()
  const { isMobile, setOpenMobile } = useSidebar()
  // Call the progress hook once at the sidebar root and thread the result
  // down — avoids a second observer/render path when SetupChecklist mounts.
  const setupProgress = useSetupProgress()
  const { hasWorkspace, workspacesLoading } = setupProgress
  const activeWorkspace = useOptionalActiveWorkspace()
  const authed = authEnabled && !!user

  // Anonymous mode (auth disabled or signed out) has no workspace concept,
  // so legacy paths render at the root and the resource nav points at
  // them directly. The progressive nav from #72 also lands here for
  // authed-but-zero-workspace users — they only see the workspace-picker
  // entry until they create one.
  const linkBase = activeWorkspace
    ? `/workspaces/${activeWorkspace.slug}`
    : ""
  const overviewPath = linkBase || "/"

  const closeMobile = () => {
    if (isMobile) setOpenMobile(false)
  }

  // Progressive disclosure: authenticated users with zero workspaces only
  // see the System group + the switcher (which handles "create workspace").
  // Once the first workspace lands the full nav unlocks. Gate on
  // workspacesLoading only (not the aggregate `loading`) so the nav
  // decision doesn't wait for the slower projects/installs queries.
  const gateNav = authed && !workspacesLoading && !hasWorkspace
  const visibleSections = SCOPED_SECTIONS.filter((s) => {
    if (gateNav && s.label !== "System") return false
    return true
  })

  // Prefix every nav item's path with the active workspace's root, or
  // — in anonymous / no-workspace mode — render the legacy bare path so
  // the link still resolves through the App.tsx legacy redirects.
  const resolvePath = (suffix: string): string => {
    if (suffix === "") return overviewPath
    return linkBase ? `${linkBase}/${suffix}` : `/${suffix}`
  }

  return (
    <Sidebar>
      <SidebarHeader className="border-b px-6 py-4">
        <Link to={overviewPath} className="flex items-center gap-2" onClick={closeMobile}>
          <OnsagerLogo size={24} />
          <span className="text-lg font-semibold">Onsager</span>
        </Link>
      </SidebarHeader>
      <SidebarContent>
        {authed && <WorkspaceSwitcher />}
        {visibleSections.map((section) => (
          <SidebarGroup key={section.label}>
            <SidebarGroupLabel>{section.label}</SidebarGroupLabel>
            <SidebarGroupContent>
              <SidebarMenu>
                {section.items.map((item) => {
                  const path = resolvePath(item.path)
                  return (
                    <SidebarMenuItem key={item.title}>
                      <SidebarMenuButton
                        render={<Link to={path} onClick={closeMobile} />}
                        isActive={
                          item.path === ""
                            ? location.pathname === path
                            : location.pathname.startsWith(path)
                        }
                      >
                        <item.icon className="h-4 w-4" />
                        <span>{item.title}</span>
                      </SidebarMenuButton>
                    </SidebarMenuItem>
                  )
                })}
              </SidebarMenu>
            </SidebarGroupContent>
          </SidebarGroup>
        ))}
        <SetupChecklist progress={setupProgress} />
      </SidebarContent>
      <SidebarFooter className="gap-1 border-t p-2">
        <UserMenu variant="row" />
        <span className="px-2 text-xs text-muted-foreground">v0.1.0</span>
      </SidebarFooter>
    </Sidebar>
  )
}
