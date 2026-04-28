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
import type { ReactNode } from "react"

const FactoryOverviewPage = lazy(() =>
  import("@/pages/FactoryOverviewPage").then((m) => ({ default: m.FactoryOverviewPage })),
)
const ArtifactsPage = lazy(() =>
  import("@/pages/ArtifactsPage").then((m) => ({ default: m.ArtifactsPage })),
)
const IssuesPage = lazy(() =>
  import("@/pages/IssuesPage").then((m) => ({ default: m.IssuesPage })),
)
const ArtifactDetailPage = lazy(() =>
  import("@/pages/ArtifactDetailPage").then((m) => ({ default: m.ArtifactDetailPage })),
)
const SpinePage = lazy(() =>
  import("@/pages/SpinePage").then((m) => ({ default: m.SpinePage })),
)
const GovernancePage = lazy(() =>
  import("@/pages/GovernancePage").then((m) => ({ default: m.GovernancePage })),
)
const SessionsPage = lazy(() =>
  import("@/pages/SessionsPage").then((m) => ({ default: m.SessionsPage })),
)
const SessionDetailPage = lazy(() =>
  import("@/pages/SessionDetailPage").then((m) => ({ default: m.SessionDetailPage })),
)
const NodesPage = lazy(() =>
  import("@/pages/NodesPage").then((m) => ({ default: m.NodesPage })),
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

const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      retry: 1,
      staleTime: 5000,
    },
  },
})

function ProtectedRoute({ children }: { children: ReactNode }) {
  const { user, loading, authEnabled } = useAuth()

  if (loading) {
    return <AppShellSkeleton />
  }

  // If auth is not enabled, allow access
  if (!authEnabled) {
    return <>{children}</>
  }

  // If auth is enabled but no user, redirect to login
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
  const { user, authEnabled } = useAuth()
  const location = useLocation()
  // Only run the onboarding redirect for authenticated users — anonymous
  // mode (auth disabled or no session) has no workspace concept to gate.
  const gateEnabled = authEnabled && !!user
  const { data, isLoading } = useQuery({
    queryKey: ["workspaces"],
    queryFn: api.listWorkspaces,
    staleTime: 30_000,
    enabled: gateEnabled,
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

  if (
    gateEnabled &&
    !isLoading &&
    workspaces.length === 0 &&
    !seen &&
    !onWorkspaces
  ) {
    return <Navigate to="/workspaces?welcome=1" replace />
  }

  return <>{children}</>
}

// Bare-path redirect: `/` → `/workspaces/<active>` if the user has a
// workspace, otherwise let `OnboardingGate` send them to the picker.
// "Active" is last-used (localStorage) when valid, else memberships[0].
// Anonymous mode (auth disabled or no user) keeps the legacy bare path
// rendering FactoryOverview directly so the existing demo flow still works.
function BarePathRedirect() {
  const { user, authEnabled, loading } = useAuth()
  const gateEnabled = authEnabled && !!user
  const { data, isLoading } = useQuery({
    queryKey: ["workspaces"],
    queryFn: api.listWorkspaces,
    staleTime: 30_000,
    enabled: gateEnabled,
  })
  const workspaces = data?.workspaces ?? []

  if (loading || (gateEnabled && isLoading)) {
    return <AppShellSkeleton />
  }

  if (!gateEnabled) {
    // Anonymous / auth-disabled: render the global Factory overview at
    // `/`, same as before. There's no workspace to scope to.
    return (
      <LazyRoute>
        <FactoryOverviewPage />
      </LazyRoute>
    )
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

// Per-resource legacy redirect: `/sessions` → `/workspaces/<active>/sessions`.
// One release of compat per spec #166; tracked by bridge-debt follow-up issue
// at land time.
function LegacyRedirect({ to }: { to: string }) {
  const { user, authEnabled } = useAuth()
  const gateEnabled = authEnabled && !!user
  const { data, isLoading } = useQuery({
    queryKey: ["workspaces"],
    queryFn: api.listWorkspaces,
    staleTime: 30_000,
    enabled: gateEnabled,
  })
  const workspaces = data?.workspaces ?? []
  const location = useLocation()

  if (gateEnabled && isLoading) return <AppShellSkeleton />

  if (!gateEnabled || workspaces.length === 0) {
    // Anonymous / zero-workspace users can't be redirected into a scoped
    // path — fall through to the picker; OnboardingGate handles welcome.
    return <Navigate to="/workspaces" replace />
  }

  const lastUsed = readLastUsedWorkspace()
  const active = workspaces.find((w) => w.slug === lastUsed) ?? workspaces[0]
  const search = location.search ?? ""
  return (
    <Navigate to={`/workspaces/${active.slug}/${to}${search}`} replace />
  )
}

// Trailing-segment redirect for `/sessions/:id`, `/artifacts/:id`,
// `/workflows/:id`. Reads the dynamic param and tacks it on.
function LegacyDetailRedirect({ resource }: { resource: string }) {
  const params = useParams()
  const id = params.id ?? ""
  return <LegacyRedirect to={`${resource}/${id}`} />
}

// Issue #82 first-run redirect: when an authed user with ≥1 workspace has
// zero workflows and lands on a workspace overview, bounce them to that
// workspace's workflows page once so the stepped hero can pitch the
// factory. Dismissed for the rest of the session via sessionStorage so
// they can navigate freely afterwards.
const WORKFLOWS_ONBOARDING_SEEN_KEY = "onsager.workflows_onboarding_seen"

function WorkflowsFirstRunGate({ children }: { children: ReactNode }) {
  const { user, authEnabled } = useAuth()
  const location = useLocation()
  const params = useParams<{ workspace?: string }>()
  const gateEnabled = authEnabled && !!user
  const slug = params.workspace ?? ""

  // Fire both queries in parallel — the old code chained `workflows` on
  // `hasWorkspace`, which serialized two RTTs. The workflows endpoint is
  // safe to call without a workspace (it filters by the current user's
  // membership); we only *use* the result when `hasWorkspace` is true.
  const { data: workspacesData } = useQuery({
    queryKey: ["workspaces"],
    queryFn: api.listWorkspaces,
    staleTime: 30_000,
    enabled: gateEnabled,
  })
  const hasWorkspace = (workspacesData?.workspaces?.length ?? 0) > 0

  const { data: workflowsData, isLoading: workflowsLoading } = useQuery({
    queryKey: ["workflows", "user"],
    queryFn: () => api.listWorkflowsForUser(),
    staleTime: 30_000,
    enabled: gateEnabled,
  })
  const workflowsCount = workflowsData?.workflows?.length ?? 0

  const seen =
    typeof window !== "undefined" &&
    window.sessionStorage.getItem(WORKFLOWS_ONBOARDING_SEEN_KEY) === "1"

  const onWorkflows = location.pathname.includes("/workflows")
  const onWorkspaceOverview =
    !!slug && location.pathname === `/workspaces/${slug}`

  useEffect(() => {
    if (onWorkflows && typeof window !== "undefined") {
      window.sessionStorage.setItem(WORKFLOWS_ONBOARDING_SEEN_KEY, "1")
    }
  }, [onWorkflows])

  if (
    gateEnabled &&
    hasWorkspace &&
    !workflowsLoading &&
    workflowsCount === 0 &&
    !seen &&
    onWorkspaceOverview
  ) {
    return <Navigate to={`/workspaces/${slug}/workflows`} replace />
  }

  return <>{children}</>
}

function AppRoutes() {
  const { user, loading, authEnabled } = useAuth()

  if (loading) {
    return <AppShellSkeleton />
  }

  return (
    <Routes>
      <Route
        path="/login"
        element={
          // If already logged in or auth disabled, redirect to dashboard
          !authEnabled || user ? <Navigate to="/" replace /> : <LoginPage />
        }
      />
      <Route
        path="/*"
        element={
          <ProtectedRoute>
            <AppLayout>
              <OnboardingGate>
                <Routes>
                  {/* Bare path: redirect to last-used workspace (or render
                      Factory overview in anonymous mode). */}
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
                        <WorkflowsFirstRunGate>
                          <Routes>
                            <Route
                              path="/"
                              element={<LazyRoute><FactoryOverviewPage /></LazyRoute>}
                            />
                            <Route
                              path="artifacts"
                              element={<LazyRoute variant="list"><ArtifactsPage /></LazyRoute>}
                            />
                            <Route
                              path="artifacts/:id"
                              element={<LazyRoute variant="detail"><ArtifactDetailPage /></LazyRoute>}
                            />
                            <Route
                              path="issues"
                              element={<LazyRoute variant="list"><IssuesPage /></LazyRoute>}
                            />
                            <Route
                              path="spine"
                              element={<LazyRoute variant="list"><SpinePage /></LazyRoute>}
                            />
                            <Route
                              path="governance"
                              element={<LazyRoute><GovernancePage /></LazyRoute>}
                            />
                            <Route
                              path="sessions"
                              element={<LazyRoute variant="list"><SessionsPage /></LazyRoute>}
                            />
                            <Route
                              path="sessions/:id"
                              element={<LazyRoute variant="detail"><SessionDetailPage /></LazyRoute>}
                            />
                            <Route
                              path="nodes"
                              element={<LazyRoute variant="list"><NodesPage /></LazyRoute>}
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
                            <Route
                              path="settings"
                              element={<LazyRoute><WorkspaceSettingsPage /></LazyRoute>}
                            />
                          </Routes>
                        </WorkflowsFirstRunGate>
                      </WorkspaceScope>
                    }
                  />

                  {/* Legacy redirects: one release of compat per spec
                      #166. Tracked by a bridge-debt follow-up issue. */}
                  <Route path="/sessions" element={<LegacyRedirect to="sessions" />} />
                  <Route
                    path="/sessions/:id"
                    element={<LegacyDetailRedirect resource="sessions" />}
                  />
                  <Route path="/nodes" element={<LegacyRedirect to="nodes" />} />
                  <Route path="/artifacts" element={<LegacyRedirect to="artifacts" />} />
                  <Route
                    path="/artifacts/:id"
                    element={<LegacyDetailRedirect resource="artifacts" />}
                  />
                  <Route path="/issues" element={<LegacyRedirect to="issues" />} />
                  <Route path="/spine" element={<LegacyRedirect to="spine" />} />
                  <Route
                    path="/governance"
                    element={<LegacyRedirect to="governance" />}
                  />
                  <Route
                    path="/workflows"
                    element={<LegacyRedirect to="workflows" />}
                  />
                  <Route
                    path="/workflows/start"
                    element={<LegacyRedirect to="workflows/start" />}
                  />
                  <Route
                    path="/workflows/:id"
                    element={<LegacyDetailRedirect resource="workflows" />}
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
