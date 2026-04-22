import { useEffect } from "react"
import { BrowserRouter, Routes, Route, Navigate, useLocation } from "react-router-dom"
import { QueryClient, QueryClientProvider, useQuery } from "@tanstack/react-query"
import { ThemeProvider } from "@/components/providers/ThemeProvider"
import { AuthProvider, useAuth } from "@/lib/auth"
import { AppLayout } from "@/components/layout/AppLayout"
import { FactoryOverviewPage } from "@/pages/FactoryOverviewPage"
import { ArtifactDetailPage } from "@/pages/ArtifactDetailPage"
import { NodesPage } from "@/pages/NodesPage"
import { SessionsPage } from "@/pages/SessionsPage"
import { SessionDetailPage } from "@/pages/SessionDetailPage"
import { LoginPage } from "@/pages/LoginPage"
import { SettingsPage } from "@/pages/SettingsPage"
import { GovernancePage } from "@/pages/GovernancePage"
import { SpinePage } from "@/pages/SpinePage"
import { ArtifactsPage } from "@/pages/ArtifactsPage"
import { WorkspacesPage } from "@/pages/WorkspacesPage"
import { WorkflowsPage } from "@/pages/WorkflowsPage"
import { WorkflowStartPage } from "@/pages/WorkflowStartPage"
import { WorkflowDetailPage } from "@/pages/WorkflowDetailPage"
import { api } from "@/lib/api"
import type { ReactNode } from "react"

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
    return (
      <div className="flex min-h-screen items-center justify-center">
        <p className="text-muted-foreground">Loading...</p>
      </div>
    )
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

  const { data: workspacesData } = useQuery({
    queryKey: ["workspaces"],
    queryFn: api.listWorkspaces,
    staleTime: 30_000,
    enabled: gateEnabled,
  })
  const hasWorkspace = (workspacesData?.tenants?.length ?? 0) > 0

  const { data: workflowsData, isLoading } = useQuery({
    queryKey: ["workflows", "user"],
    queryFn: () => api.listWorkflowsForUser(),
    staleTime: 30_000,
    enabled: gateEnabled && hasWorkspace,
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
    !isLoading &&
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
    return (
      <div className="flex min-h-screen items-center justify-center">
        <p className="text-muted-foreground">Loading...</p>
      </div>
    )
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
                    <Route path="/" element={<FactoryOverviewPage />} />
                    <Route path="/artifacts" element={<ArtifactsPage />} />
                    <Route path="/artifacts/:id" element={<ArtifactDetailPage />} />
                    <Route path="/spine" element={<SpinePage />} />
                    <Route path="/governance" element={<GovernancePage />} />
                    <Route path="/sessions" element={<SessionsPage />} />
                    <Route path="/sessions/:id" element={<SessionDetailPage />} />
                    <Route path="/nodes" element={<NodesPage />} />
                    <Route path="/workspaces" element={<WorkspacesPage />} />
                    <Route path="/workflows" element={<WorkflowsPage />} />
                    <Route path="/workflows/start" element={<WorkflowStartPage />} />
                    <Route path="/workflows/:id" element={<WorkflowDetailPage />} />
                    <Route path="/settings" element={<SettingsPage />} />
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
