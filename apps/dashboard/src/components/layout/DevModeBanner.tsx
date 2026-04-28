import { TerminalSquare } from "lucide-react"
import { useAuth } from "@/lib/auth"

/**
 * Persistent banner shown only when the active session was minted via
 * dev-login (issue #193). It's the discoverability surface for an
 * otherwise-invisible primitive — without it, a returning user can land
 * on the dashboard already authenticated as `${USER}@local` with no
 * idea why their account looks the way it does.
 *
 * The banner is mounted unconditionally near the top of the app shell
 * and self-hides when `session_kind` is anything other than `"dev"`. We
 * keep it inside `<ProtectedRoute>` so it never paints on `/login`.
 */
export function DevModeBanner() {
  const { user, sessionKind } = useAuth()
  if (sessionKind !== "dev" || !user) return null

  return (
    <div
      role="status"
      aria-live="polite"
      className="flex items-center gap-2 border-b border-amber-300 bg-amber-50 px-4 py-1.5 text-xs text-amber-900 dark:border-amber-700/60 dark:bg-amber-950/60 dark:text-amber-100"
    >
      <TerminalSquare className="h-3.5 w-3.5 shrink-0" />
      <span className="truncate">
        Dev mode — signed in as{" "}
        <span className="font-mono font-medium">{user.github_login}</span>.
        Build with <span className="font-mono">--release</span> to disable.
      </span>
    </div>
  )
}
