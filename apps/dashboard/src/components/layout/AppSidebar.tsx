import { GitBranch, MessageSquare } from "lucide-react"
import { OnsagerLogo } from "./OnsagerLogo"
import { UserMenu } from "./UserMenu"
import { Link, useLocation } from "react-router-dom"
import {
  Sidebar,
  SidebarContent,
  SidebarFooter,
  SidebarGroup,
  SidebarGroupContent,
  SidebarHeader,
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
  useSidebar,
} from "@/components/ui/sidebar"
import { useOptionalActiveWorkspace } from "@/lib/workspace"
import { useSetupProgress } from "@/hooks/useSetupProgress"
import { WorkspaceSwitcher } from "@/components/workspaces/WorkspaceSwitcher"

interface NavItem {
  title: string
  icon: typeof GitBranch
  /** Path relative to the active workspace root (`/workspaces/<slug>/...`). */
  path: string
}

// Two-surface IA per spec #289: Chat (R&D — design workflows by talking
// to the agent) and Workflows (production — library of deployed
// automations). Everything else (Issues, Artifacts, Spine, Governance,
// Sessions, Nodes, Factory Overview, Settings) stays reachable via the
// ⌘K command palette and direct URL during the rollout — PR 1 only
// changes what shows up in the sidebar nav.
const SCOPED_ITEMS: NavItem[] = [
  { title: "Chat", icon: MessageSquare, path: "chat" },
  { title: "Workflows", icon: GitBranch, path: "workflows" },
]

export function AppSidebar() {
  const location = useLocation()
  const { isMobile, setOpenMobile } = useSidebar()
  // `useSetupProgress` drives the progressive nav disclosure below — the
  // sidebar hides scoped items until the user has at least one workspace.
  // (The sidebar checklist that previously shared this hook was deleted
  // in spec #403.)
  const { hasWorkspace, workspacesLoading } = useSetupProgress()
  const activeWorkspace = useOptionalActiveWorkspace()

  // Auth is always-on as of #193. Pages outside a scoped route (the
  // workspace picker, account settings) leave `activeWorkspace` null
  // and route nav back to `/workspaces` so the user picks one.
  const linkBase = activeWorkspace
    ? `/workspaces/${activeWorkspace.slug}`
    : null
  const overviewPath = linkBase ?? "/workspaces"

  const closeMobile = () => {
    if (isMobile) setOpenMobile(false)
  }

  // Progressive disclosure: users with zero workspaces see only the
  // switcher (which handles "create workspace"). Once the first
  // workspace lands the nav items unlock. Gate on workspacesLoading
  // only (not the aggregate `loading`) so the nav decision doesn't
  // wait for the slower projects/installs queries.
  const gateNav = !workspacesLoading && !hasWorkspace

  return (
    <Sidebar>
      <SidebarHeader className="border-b px-6 py-4">
        <Link to={overviewPath} className="flex items-center gap-2" onClick={closeMobile}>
          <OnsagerLogo size={24} />
          <span className="text-lg font-semibold">Onsager</span>
        </Link>
      </SidebarHeader>
      <SidebarContent>
        <WorkspaceSwitcher />
        {!gateNav && (
          <SidebarGroup>
            <SidebarGroupContent>
              <SidebarMenu>
                {SCOPED_ITEMS.map((item) => {
                  const path = linkBase
                    ? `${linkBase}/${item.path}`
                    : "/workspaces"
                  // Outside a scoped route every item shares the
                  // `/workspaces` picker fallback — none of them is
                  // really "this page", so don't highlight any.
                  const isActive =
                    linkBase != null &&
                    (location.pathname === path ||
                      location.pathname.startsWith(`${path}/`))
                  return (
                    <SidebarMenuItem key={item.title}>
                      <SidebarMenuButton
                        render={<Link to={path} onClick={closeMobile} />}
                        isActive={isActive}
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
        )}
      </SidebarContent>
      <SidebarFooter className="gap-1 border-t p-2">
        <UserMenu variant="row" />
        <span className="px-2 text-xs text-muted-foreground">v0.1.0</span>
      </SidebarFooter>
    </Sidebar>
  )
}
