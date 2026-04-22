import { lazy, Suspense, useEffect } from "react"
import { BrowserRouter, Routes, Route, Navigate, useLocation } from "react-router-dom"
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
import type { ReactNode } from "react"

const FactoryOverviewPage = lazy(() =>
  import("@/pages/FactoryOverviewPage").then((m) => ({ default: m.FactoryOverviewPage })),
)
const ArtifactsPage = lazy(() =>
  import("@/pages/ArtifactsPage").then((m) => ({ default: m.ArtifactsPage })),
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
  const workspaces = data?.tenants ?? []
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

// Issue #82 first-run redirect: when an authed user with ≥1 workspace has
// zero workflows and lands on /, bounce them to /workflows once so the
// stepped hero can pitch the factory. Dismissed for the rest of the session
// via sessionStorage so they can navigate freely afterwards.
const WORKFLOWS_ONBOARDING_SEEN_KEY = "onsager.workflows_onboarding_seen"

function WorkflowsFirstRunGate({ children }: { children: ReactNode }) {
  const { user, authEnabled } = useAuth()
  const location = useLocation()
  const gateEnabled = authEnabled && !!user

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
  const hasWorkspace = (workspacesData?.tenants?.length ?? 0) > 0

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

  const onWorkflows =
    location.pathname === "/workflows" ||
    location.pathname.startsWith("/workflows/")

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
    location.pathname === "/"
  ) {
    return <Navigate to="/workflows" replace />
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
                <WorkflowsFirstRunGate>
                  <Routes>
                    <Route
                      path="/"
                      element={<LazyRoute><FactoryOverviewPage /></LazyRoute>}
                    />
                    <Route
                      path="/artifacts"
                      element={<LazyRoute variant="list"><ArtifactsPage /></LazyRoute>}
                    />
                    <Route
                      path="/artifacts/:id"
                      element={<LazyRoute variant="detail"><ArtifactDetailPage /></LazyRoute>}
                    />
                    <Route
                      path="/spine"
                      element={<LazyRoute variant="list"><SpinePage /></LazyRoute>}
                    />
                    <Route
                      path="/governance"
                      element={<LazyRoute><GovernancePage /></LazyRoute>}
                    />
                    <Route
                      path="/sessions"
                      element={<LazyRoute variant="list"><SessionsPage /></LazyRoute>}
                    />
                    <Route
                      path="/sessions/:id"
                      element={<LazyRoute variant="detail"><SessionDetailPage /></LazyRoute>}
                    />
                    <Route
                      path="/nodes"
                      element={<LazyRoute variant="list"><NodesPage /></LazyRoute>}
                    />
                    <Route
                      path="/workspaces"
                      element={<LazyRoute variant="list"><WorkspacesPage /></LazyRoute>}
                    />
                    <Route
                      path="/workflows"
                      element={<LazyRoute variant="list"><WorkflowsPage /></LazyRoute>}
                    />
                    <Route
                      path="/workflows/start"
                      element={<LazyRoute><WorkflowStartPage /></LazyRoute>}
                    />
                    <Route
                      path="/workflows/:id"
                      element={<LazyRoute variant="detail"><WorkflowDetailPage /></LazyRoute>}
                    />
                    <Route
                      path="/settings"
                      element={<LazyRoute><SettingsPage /></LazyRoute>}
                    />
                  </Routes>
                </WorkflowsFirstRunGate>
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
