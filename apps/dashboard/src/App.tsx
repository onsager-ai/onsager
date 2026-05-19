import { lazy, Suspense } from "react"
import { BrowserRouter, Routes, Route, Navigate, useParams } from "react-router-dom"
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
import { WorkspaceScope } from "@/lib/workspace"
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
const LandingPage = lazy(() =>
  import("@/pages/LandingPage").then((m) => ({ default: m.LandingPage })),
)
const ShowcaseDogfoodPage = lazy(() =>
  import("@/pages/ShowcaseDogfoodPage").then((m) => ({
    default: m.ShowcaseDogfoodPage,
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

  // Public landing surface (#408 location 4) — must render before the
  // auth probe resolves so a first-contact visit isn't gated on
  // `/api/auth/me`. Matched before the auth-loading branch.
  const landingRoute = (
    <Route
      path="/landing"
      element={
        <LazyRoute>
          <LandingPage />
        </LazyRoute>
      }
    />
  )

  // Public Dogfood showcase (spec #407). Same auth-pre-empt as
  // /landing: an evaluator clicking "see live runs ↗" from the OSS
  // README hits this route directly, with no session.
  const showcaseRoute = (
    <Route
      path="/showcase/dogfood"
      element={
        <LazyRoute>
          <ShowcaseDogfoodPage />
        </LazyRoute>
      }
    />
  )

  if (loading) {
    return (
      <Routes>
        {landingRoute}
        {showcaseRoute}
        <Route path="*" element={<AppShellSkeleton />} />
      </Routes>
    )
  }

  return (
    <Routes>
      {landingRoute}
      {showcaseRoute}
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
              <Routes>
                {/* Bare path: redirect to /chat (spec #398). ChatPage
                    is the universal entry — workspace-less users see
                    the FTUE empty state; workspace users see the same
                    surface with their last-used workspace in scope. */}
                <Route path="/" element={<Navigate to="/chat" replace />} />

                {/* Top-level Chat (spec #398). The same surface lives at
                    `/workspaces/:slug/chat` for users who deep-link a
                    specific workspace; this unscoped mount is the
                    universal landing. */}
                <Route
                  path="/chat"
                  element={<LazyRoute><ChatPage /></LazyRoute>}
                />

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
