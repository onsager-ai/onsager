import { useEffect, useState } from "react"
import { TerminalSquare } from "lucide-react"
import { OnsagerLogo } from "@/components/layout/OnsagerLogo"
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/card"
import { Button, buttonVariants } from "@/components/ui/button"
import { api, ApiError } from "@/lib/api"

/**
 * Login screen.
 *
 * Renders the GitHub OAuth entry-point unconditionally. The "Dev Login"
 * button is conditional: we probe `/api/auth/dev-login` with a HEAD-style
 * request and only render the button when the route exists, which is the
 * case in `cargo build` (debug) but not `cargo build --release`. Issue
 * #193 — anonymous mode was deleted; dev-login replaces it as the
 * SSO-free path for local development.
 */
export function LoginPage() {
  const [devAvailable, setDevAvailable] = useState(false)
  const [devLoggingIn, setDevLoggingIn] = useState(false)
  const [devError, setDevError] = useState<string | null>(null)

  // Detect whether the server has the dev-login route. A bare GET is
  // enough — debug builds reply 405 (route registered, wrong method);
  // release builds reply 404 (route absent). Any non-404 means present.
  useEffect(() => {
    let cancelled = false
    fetch("/api/auth/dev-login", { method: "GET" })
      .then((r) => {
        if (!cancelled) setDevAvailable(r.status !== 404)
      })
      .catch(() => {
        // Network failure — leave the button hidden rather than
        // letting a backend outage suggest dev-login is available.
      })
    return () => {
      cancelled = true
    }
  }, [])

  async function handleDevLogin() {
    setDevLoggingIn(true)
    setDevError(null)
    try {
      await api.devLogin()
      // Cookie is now set; reload so AuthProvider re-fetches /me and the
      // app shell renders behind the new session.
      window.location.href = "/"
    } catch (err) {
      const msg =
        err instanceof ApiError
          ? `dev-login failed (${err.status})`
          : "dev-login failed"
      setDevError(msg)
      setDevLoggingIn(false)
    }
  }

  return (
    <div className="flex min-h-screen items-center justify-center bg-background p-4">
      <Card className="w-full max-w-sm">
        <CardHeader className="text-center">
          <div className="mx-auto mb-2 flex h-12 w-12 items-center justify-center rounded-lg bg-primary/10">
            <OnsagerLogo size={24} className="text-primary" />
          </div>
          <CardTitle className="text-xl">Onsager</CardTitle>
          <CardDescription>
            Sign in to manage your distributed agent sessions.
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-3">
          <a
            href="/api/auth/github"
            className={buttonVariants({ size: "lg", className: "w-full" })}
          >
            <svg className="mr-2 h-5 w-5" viewBox="0 0 24 24" fill="currentColor">
              <path d="M12 0c-6.626 0-12 5.373-12 12 0 5.302 3.438 9.8 8.207 11.387.599.111.793-.261.793-.577v-2.234c-3.338.726-4.033-1.416-4.033-1.416-.546-1.387-1.333-1.756-1.333-1.756-1.089-.745.083-.729.083-.729 1.205.084 1.839 1.237 1.839 1.237 1.07 1.834 2.807 1.304 3.492.997.107-.775.418-1.305.762-1.604-2.665-.305-5.467-1.334-5.467-5.931 0-1.311.469-2.381 1.236-3.221-.124-.303-.535-1.524.117-3.176 0 0 1.008-.322 3.301 1.23.957-.266 1.983-.399 3.003-.404 1.02.005 2.047.138 3.006.404 2.291-1.552 3.297-1.23 3.297-1.23.653 1.653.242 2.874.118 3.176.77.84 1.235 1.911 1.235 3.221 0 4.609-2.807 5.624-5.479 5.921.43.372.823 1.102.823 2.222v3.293c0 .319.192.694.801.576 4.765-1.589 8.199-6.086 8.199-11.386 0-6.627-5.373-12-12-12z" />
            </svg>
            Sign in with GitHub
          </a>
          {devAvailable && (
            <>
              <div className="relative my-1 flex items-center">
                <div className="flex-1 border-t border-border" />
                <span className="px-2 text-[10px] uppercase tracking-wider text-muted-foreground">
                  Local dev only
                </span>
                <div className="flex-1 border-t border-border" />
              </div>
              <Button
                type="button"
                variant="outline"
                size="lg"
                className="w-full"
                disabled={devLoggingIn}
                onClick={handleDevLogin}
              >
                <TerminalSquare className="mr-2 h-5 w-5" />
                {devLoggingIn ? "Signing in…" : "Dev Login"}
              </Button>
              {devError && (
                <p className="text-center text-xs text-destructive">
                  {devError}
                </p>
              )}
              <p className="text-center text-[11px] text-muted-foreground">
                Available in debug builds only. Build with{" "}
                <span className="font-mono">--release</span> to disable.
              </p>
            </>
          )}
        </CardContent>
      </Card>
    </div>
  )
}
