import { lazy, Suspense, useEffect } from "react"
import { BrowserRouter, Routes, Route, Navigate, useLocation, useParams } from "react-router-dom"
import { QueryClient, QueryClientProvider, useQuery } from "@tanstack/react-query"
import { ThemeProvider } from "@/components/providers/ThemeProvider"
import { AuthProvider, useAuth } from "@/lib/auth"
import { AppLayout } from "@/components/layout/AppLayout"
import { AppShellSkeleton } from "@/components/layout/AppShellSkeleton"
import { PageSkeleton } from "@/components/layout/PageSkeleton"
// Login stays eager so the unauthenticated first-paint path doesn't
// pay for an extra chunk fetch. Every other page is lazy.
import { LoginPage } from "@/pages/LoginPage"
import { api } from "@/lib/api"
import { readLastUsedWorkspace, WorkspaceScope } from "@/lib/workspace"
import { DevModeBanner } from "@/components/layout/DevModeBanner"
import type { ReactNode } from "react"

const IssueDetailPage = lazy(() =>
  import("@/pages/IssueDetailPage").then((m) => ({ default: m.IssueDetailPage })),
)
const ArtifactDetailPage = lazy(() =>
  import("@/pages/ArtifactDetailPage").then((m) => ({ default: m.ArtifactDetailPage })),
)
const WorkspacesPage = lazy(() =>
  import("@/pages/WorkspacesPage").then((m) => ({ default: m.WorkspacesPage })),
)
const WorkflowsPage = lazy(() =>
  import("@/pages/WorkflowsPage").then((m) => ({ default: m.WorkflowsPage })),
)
const WorkflowStartPage = lazy(() =>
  import("@/pages/WorkflowStartPage").then((m) => ({ default: m.WorkflowStartPage })),
)
const WorkflowDetailPage = lazy(() =>
  import("@/pages/WorkflowDetailPage").then((m) => ({ default: m.WorkflowDetailPage })),
)
const RunDetailPage = lazy(() =>
  import("@/pages/RunDetailPage").then((m) => ({ default: m.RunDetailPage })),
)
const SettingsPage = lazy(() =>
  import("@/pages/SettingsPage").then((m) => ({ default: m.SettingsPage })),
)
const WorkspaceSettingsPage = lazy(() =>
  import("@/pages/WorkspaceSettingsPage").then((m) => ({
    default: m.WorkspaceSettingsPage,
  })),
)
const PersonalAccessTokensPage = lazy(() =>
  import("@/pages/PersonalAccessTokensPage").then((m) => ({
    default: m.PersonalAccessTokensPage,
  })),
)
const ChatPage = lazy(() =>
  import("@/pages/ChatPage").then((m) => ({ default: m.ChatPage })),
)

const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      retry: 1,
      staleTime: 5000,
    },
  },
})

function ProtectedRoute({ children }: { children: ReactNode }) {
  const { user, loading } = useAuth()

  if (loading) {
    return <AppShellSkeleton />
  }

  // Auth is always-on as of #193 — anonymous mode is gone. Either the
  // user has a real session (GitHub OAuth or dev-login in debug builds)
  // or they bounce to /login.
  if (!user) {
    return <Navigate to="/login" replace />
  }

  return <>{children}</>
}

// Wraps a lazy page's element in a Suspense with a variant-appropriate
// skeleton. Chunk loads are typically sub-second but on slow connections
// this keeps the main area non-empty while the JS streams in.
function LazyRoute({
  variant = "default",
  children,
}: {
  variant?: "list" | "detail" | "default"
  children: ReactNode
}) {
  return <Suspense fallback={<PageSkeleton variant={variant} />}>{children}</Suspense>
}

// Redirects users with zero workspaces to /workspaces?welcome=1 on their
// first visit of the session. Once we've shown the welcome hero, mark the
// onboarding as seen so the user can navigate freely without being bounced
// back — they'll still see the empty-state banner on pages that render one.
const ONBOARDING_SEEN_KEY = "onsager.onboarding_seen"

function OnboardingGate({ children }: { children: ReactNode }) {
  const location = useLocation()
  // ProtectedRoute already enforces an authenticated user, so the query
  // can fire unconditionally — there's no anonymous branch to skip.
  const { data, isLoading } = useQuery({
    queryKey: ["workspaces"],
    queryFn: api.listWorkspaces,
    staleTime: 30_000,
  })
  const workspaces = data?.workspaces ?? []
  const seen =
    typeof window !== "undefined" &&
    window.sessionStorage.getItem(ONBOARDING_SEEN_KEY) === "1"

  const onWorkspaces =
    location.pathname === "/workspaces" ||
    location.pathname.startsWith("/workspaces/")

  useEffect(() => {
    if (onWorkspaces && typeof window !== "undefined") {
      window.sessionStorage.setItem(ONBOARDING_SEEN_KEY, "1")
    }
  }, [onWorkspaces])

  if (!isLoading && workspaces.length === 0 && !seen && !onWorkspaces) {
    return <Navigate to="/workspaces?welcome=1" replace />
  }

  return <>{children}</>
}

// Bare-path redirect: `/` → `/workspaces/<active>` if the user has a
// workspace, otherwise let `OnboardingGate` send them to the picker.
// "Active" is last-used (localStorage) when valid, else memberships[0].
function BarePathRedirect() {
  const { loading } = useAuth()
  const { data, isLoading } = useQuery({
    queryKey: ["workspaces"],
    queryFn: api.listWorkspaces,
    staleTime: 30_000,
  })
  const workspaces = data?.workspaces ?? []

  if (loading || isLoading) {
    return <AppShellSkeleton />
  }

  if (workspaces.length === 0) {
    // OnboardingGate will catch this on the next render; keep the user
    // visible while it does so the screen isn't blank.
    return <Navigate to="/workspaces?welcome=1" replace />
  }

  const lastUsed = readLastUsedWorkspace()
  const active = workspaces.find((w) => w.slug === lastUsed) ?? workspaces[0]
  return <Navigate to={`/workspaces/${active.slug}`} replace />
}

// Builds an absolute workspace-scoped redirect from a relative suffix.
// Used by the bookmark-preserving redirects (spec #306) so each redirect
// doesn't have to thread the :workspace param manually.
function WorkspaceRedirect({ to }: { to: string }) {
  const { workspace } = useParams<{ workspace: string }>()
  return <Navigate to={`/workspaces/${workspace}/${to}`} replace />
}

// Resolves a legacy `sessions/:id` bookmark to its run. Fetches the session
// and redirects to /runs/:artifact_id when the session is linked to a run;
// falls back to /workflows when the session has no run or isn't found.
function SessionIdRedirect() {
  const { workspace, id } = useParams<{ workspace: string; id: string }>()
  const { data, isLoading } = useQuery({
    queryKey: ["session", id],
    queryFn: () => api.getSession(id!),
    retry: 1,
    staleTime: 0,
  })

  if (isLoading) return null

  const artifactId = data?.session?.artifact_id
  if (artifactId) {
    return <Navigate to={`/workspaces/${workspace}/runs/${artifactId}`} replace />
  }
  return <Navigate to={`/workspaces/${workspace}/workflows`} replace />
}

function AppRoutes() {
  const { user, loading } = useAuth()

  if (loading) {
    return <AppShellSkeleton />
  }

  return (
    <Routes>
      <Route
        path="/login"
        element={user ? <Navigate to="/" replace /> : <LoginPage />}
      />
      <Route
        path="/*"
        element={
          <ProtectedRoute>
            <DevModeBanner />
            <AppLayout>
              <OnboardingGate>
                <Routes>
                  {/* Bare path: redirect to last-used workspace. With auth
                      always-on (#193), every user has a membership context. */}
                  <Route path="/" element={<BarePathRedirect />} />

                  {/* Workspace picker / list. Stays unscoped — the user
                      lands here when they have zero workspaces or want to
                      switch. */}
                  <Route
                    path="/workspaces"
                    element={<LazyRoute variant="list"><WorkspacesPage /></LazyRoute>}
                  />

                  {/* Account-wide settings (profile, PAT list). */}
                  <Route
                    path="/settings"
                    element={<LazyRoute><SettingsPage /></LazyRoute>}
                  />
                  <Route
                    path="/settings/tokens"
                    element={<LazyRoute><PersonalAccessTokensPage /></LazyRoute>}
                  />

                  {/* Workspace-scoped routes. WorkspaceScope validates the
                      slug and supplies WorkspaceContext. */}
                  <Route
                    path="/workspaces/:workspace/*"
                    element={
                      <WorkspaceScope>
                        <Routes>
                          {/* Bare workspace path → workflows (spec #306). */}
                          <Route
                            index
                            element={<Navigate to="workflows" replace />}
                          />
                          <Route
                            path="chat"
                            element={<LazyRoute><ChatPage /></LazyRoute>}
                          />
                          {/* Bookmark redirects (spec #306). Preserves any
                              in-the-wild URLs after top-level surfaces were
                              demoted in the two-surface IA (#289). */}
                          <Route path="sessions" element={<WorkspaceRedirect to="workflows" />} />
                          <Route path="sessions/:id" element={<SessionIdRedirect />} />
                          <Route path="artifacts" element={<WorkspaceRedirect to="workflows" />} />
                          <Route path="spine" element={<WorkspaceRedirect to="workflows" />} />
                          <Route path="governance" element={<WorkspaceRedirect to="settings#governance-audit" />} />
                          <Route path="issues" element={<WorkspaceRedirect to="workflows" />} />
                          <Route
                            path="issues/:projectId/:number"
                            element={<LazyRoute variant="detail"><IssueDetailPage /></LazyRoute>}
                          />
                          <Route path="nodes" element={<WorkspaceRedirect to="settings#infrastructure" />} />
                          <Route
                            path="artifacts/:id"
                            element={<LazyRoute variant="detail"><ArtifactDetailPage /></LazyRoute>}
                          />
                          <Route
                            path="workflows"
                            element={<LazyRoute variant="list"><WorkflowsPage /></LazyRoute>}
                          />
                          <Route
                            path="workflows/start"
                            element={<LazyRoute><WorkflowStartPage /></LazyRoute>}
                          />
                          <Route
                            path="workflows/:id"
                            element={<LazyRoute variant="detail"><WorkflowDetailPage /></LazyRoute>}
                          />
                          {/* Run detail hub (#303). Flat route — runs
                              have unique IDs (artifact_id), so they
                              don't need to be nested under workflow. */}
                          <Route
                            path="runs/:runId"
                            element={<LazyRoute variant="detail"><RunDetailPage /></LazyRoute>}
                          />
                          <Route
                            path="settings"
                            element={<LazyRoute><WorkspaceSettingsPage /></LazyRoute>}
                          />
                        </Routes>
                      </WorkspaceScope>
                    }
                  />
                </Routes>
              </OnboardingGate>
            </AppLayout>
          </ProtectedRoute>
        }
      />
    </Routes>
  )
}

function App() {
  return (
    <ThemeProvider>
      <QueryClientProvider client={queryClient}>
        <BrowserRouter>
          <AuthProvider>
            <AppRoutes />
          </AuthProvider>
        </BrowserRouter>
      </QueryClientProvider>
    </ThemeProvider>
  )
}

export default App
