import { ChevronsUpDown, LogOut, Monitor, Moon, Settings, Sun, User as UserIcon } from "lucide-react"
import { Link } from "react-router-dom"
import { useTheme } from "next-themes"
import { Button } from "@/components/ui/button"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { useAuth } from "@/lib/auth"

// `icon`: legacy circular avatar button (kept for any non-sidebar use).
// `row`:  full-width pill used in the sidebar footer — avatar + label +
//         disclosure chevron. Mirrors Slack/Linear/Notion's account block.
type UserMenuVariant = "icon" | "row"

export interface UserMenuProps {
  variant?: UserMenuVariant
}

export function UserMenu({ variant = "icon" }: UserMenuProps) {
  const { user, authEnabled, logout } = useAuth()
  const { setTheme } = useTheme()

  const initial = user?.github_name?.[0] ?? user?.github_login?.[0] ?? "?"
  const label = user?.github_name ?? user?.github_login ?? "Account"
  const sublabel = authEnabled && user?.github_login ? `@${user.github_login}` : undefined

  const avatar =
    authEnabled && user?.github_avatar_url ? (
      <img src={user.github_avatar_url} alt={label} className="h-7 w-7 shrink-0 rounded-full" />
    ) : (
      <div className="flex h-7 w-7 shrink-0 items-center justify-center rounded-full bg-muted text-xs font-medium uppercase text-muted-foreground">
        {authEnabled ? initial : <UserIcon className="h-4 w-4" />}
      </div>
    )

  const trigger =
    variant === "row" ? (
      <Button
        variant="ghost"
        className="h-auto w-full items-center justify-start gap-2 px-2 py-1.5 text-left"
        aria-label="Open user menu"
      >
        {avatar}
        <div className="min-w-0 flex-1">
          <div className="truncate text-sm font-medium">{label}</div>
          {sublabel && (
            <div className="truncate text-xs text-muted-foreground">{sublabel}</div>
          )}
        </div>
        <ChevronsUpDown className="h-4 w-4 shrink-0 text-muted-foreground" />
      </Button>
    ) : (
      <Button
        variant="ghost"
        size="icon"
        className="h-9 w-9 rounded-full p-0"
        aria-label="Open user menu"
      >
        {avatar}
      </Button>
    )

  return (
    <DropdownMenu>
      <DropdownMenuTrigger render={trigger} />
      <DropdownMenuContent
        align={variant === "row" ? "start" : "end"}
        side={variant === "row" ? "top" : "bottom"}
        className="w-56"
      >
        {authEnabled && user ? (
          <>
            <DropdownMenuLabel className="flex flex-col gap-0.5 px-2 py-1.5">
              <span className="truncate text-sm font-medium text-foreground">
                {user.github_name ?? user.github_login}
              </span>
              {user.github_name && (
                <span className="truncate text-xs text-muted-foreground">
                  @{user.github_login}
                </span>
              )}
            </DropdownMenuLabel>
            <DropdownMenuSeparator />
          </>
        ) : null}
        <DropdownMenuItem render={<Link to="/settings" />}>
          <Settings className="mr-2 h-4 w-4" />
          Settings
        </DropdownMenuItem>
        <DropdownMenuSeparator />
        <DropdownMenuLabel>Theme</DropdownMenuLabel>
        <DropdownMenuItem onClick={() => setTheme("light")}>
          <Sun className="mr-2 h-4 w-4" />
          Light
        </DropdownMenuItem>
        <DropdownMenuItem onClick={() => setTheme("dark")}>
          <Moon className="mr-2 h-4 w-4" />
          Dark
        </DropdownMenuItem>
        <DropdownMenuItem onClick={() => setTheme("system")}>
          <Monitor className="mr-2 h-4 w-4" />
          System
        </DropdownMenuItem>
        {authEnabled && user && (
          <>
            <DropdownMenuSeparator />
            <DropdownMenuItem variant="destructive" onClick={logout}>
              <LogOut className="mr-2 h-4 w-4" />
              Sign out
            </DropdownMenuItem>
          </>
        )}
      </DropdownMenuContent>
    </DropdownMenu>
  )
}
