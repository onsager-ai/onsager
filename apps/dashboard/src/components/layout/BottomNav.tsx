import type { ComponentType, SVGProps } from "react"
import { LayoutDashboard, Server, Terminal, Settings } from "lucide-react"
import { Link, useLocation } from "react-router-dom"
import { cn } from "@/lib/utils"

const navItems = [
  { title: "Dashboard", icon: LayoutDashboard, path: "/" },
  { title: "Sessions", icon: Terminal, path: "/sessions" },
  { title: "Nodes", icon: Server, path: "/nodes" },
  { title: "Settings", icon: Settings, path: "/settings" },
]

function NavLink({ item }: { item: { title: string; icon: ComponentType<SVGProps<SVGSVGElement>>; path: string } }) {
  const location = useLocation()
  const isActive =
    item.path === "/"
      ? location.pathname === "/"
      : location.pathname.startsWith(item.path)
  return (
    <Link
      to={item.path}
      className={cn(
        "flex flex-1 flex-col items-center justify-center gap-0.5 text-[10px] font-medium transition-colors",
        isActive
          ? "text-primary"
          : "text-muted-foreground active:text-primary"
      )}
    >
      <item.icon className={cn("h-5 w-5", isActive && "stroke-[2.5]")} />
      <span>{item.title}</span>
    </Link>
  )
}

export function BottomNav() {
  return (
    <nav className="fixed bottom-0 left-0 right-0 z-50 border-t bg-background/95 backdrop-blur-sm pb-[env(safe-area-inset-bottom)] md:hidden">
      <div className="flex h-14 items-stretch">
        {navItems.map((item) => (
          <NavLink key={item.title} item={item} />
        ))}
      </div>
    </nav>
  )
}
