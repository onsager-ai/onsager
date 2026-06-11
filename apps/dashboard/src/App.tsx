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
import { WorkspaceScope, useActiveWorkspace } from "@/lib/workspace"
import { DevModeBanner } from "@/components/layout/DevModeBanner"
import type { ReactNode } from "react"

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
const SpecPlansPage = lazy(() =>
  import("@/pages/SpecPlansPage").then((m) => ({ default: m.SpecPlansPage })),
)
const CreateSpecPlanPage = lazy(() =>
  import("@/pages/CreateSpecPlanPage").then((m) => ({
    default: m.CreateSpecPlanPage,
  })),
)
const RunsPage = lazy(() =>
  import("@/pages/RunsPage").then((m) => ({ default: m.RunsPage })),
)
const ActivityPage = lazy(() =>
  import("@/pages/ActivityPage").then((m) => ({ default: m.ActivityPage })),
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
// ChatPage is no longer mounted as a route (#457 / ADR 0019). It stays
// in the codebase as a component for ADR 0020's portable global chat
// surface — but App.tsx doesn't lazy-import it any more.
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

// Bare-path redirect (#453 / ADR 0019). Lands the signed-in user on
// `/workspaces/:slug/workflows` for their first membership workspace;
// users with zero workspaces bounce to `/workspaces` (the picker /
// FTUE surface). Mirrors the auth-aware behaviour the legacy
// `/` → `/chat` redirect relied on.
function BarePathRedirect() {
  const { data, isLoading } = useQuery({
    queryKey: ["workspaces"],
    queryFn: api.listWorkspaces,
    staleTime: 30_000,
    retry: 1,
  })
  if (isLoading) return null
  const first = data?.workspaces?.[0]
  if (first) {
    return <Navigate to={`/workspaces/${first.slug}/workflows`} replace />
  }
  return <Navigate to="/workspaces" replace />
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

// Retires IssueDetailPage (ADR 0021 / #492): `/issues/:projectId/:number`
// resolves to the canonical `/artifacts/:id` for the `github_issue`
// artifact via the same `external_ref` join IssueDetailPage performed.
// Un-ingested issues (no artifact yet) or a failed lookup fall back to
// Workflows, consistent with SessionIdRedirect.
export function IssueArtifactRedirect() {
  const { projectId = "", number = "" } = useParams<{
    projectId: string
    number: string
  }>()
  const workspace = useActiveWorkspace()
  const numberValid = /^\d+$/.test(number)
  const { data, isLoading } = useQuery({
    queryKey: ["artifacts", workspace.id, "github_issue", projectId],
    queryFn: () =>
      api.getArtifacts(workspace.id, {
        kind: "github_issue",
        project_id: projectId,
      }),
    enabled: !!projectId && numberValid,
    retry: 1,
    staleTime: 0,
  })

  if (!projectId || !numberValid) {
    return <Navigate to={`/workspaces/${workspace.slug}/workflows`} replace />
  }
  if (isLoading) return null

  const externalRef = `github:project:${projectId}:issue:${Number.parseInt(
    number,
    10,
  )}`
  const match = data?.artifacts.find((a) => a.external_ref === externalRef)
  if (match) {
    return (
      <Navigate
        to={`/workspaces/${workspace.slug}/artifacts/${match.id}`}
        replace
      />
    )
  }
  return <Navigate to={`/workspaces/${workspace.slug}/workflows`} replace />
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
                {/* Bare path: redirect to the user's first workspace's
                    Workflows tab (#453 / ADR 0019). Users with zero
                    workspaces bounce to the picker / FTUE surface. The
                    legacy `/` → `/chat` route was retired alongside the
                    chat page-route (#457). */}
                <Route path="/" element={<BarePathRedirect />} />

                {/* `/chat` retired as a page route (#457 / ADR 0019).
                    Chat becomes a portable global component per ADR 0020;
                    the address-bar entry redirects to the user's
                    workspace overview so bookmarks don't 404. */}
                <Route
                  path="/chat"
                  element={<BarePathRedirect />}
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
                        {/* Workspace-scoped chat retired as a page route
                            (#457 / ADR 0019). Redirect preserves bookmarks. */}
                        <Route path="chat" element={<WorkspaceRedirect to="workflows" />} />
                        {/* Bookmark redirects (spec #306). Preserves any
                            in-the-wild URLs after top-level surfaces were
                            demoted. */}
                        <Route path="sessions" element={<WorkspaceRedirect to="workflows" />} />
                        <Route path="sessions/:id" element={<SessionIdRedirect />} />
                        <Route path="artifacts" element={<WorkspaceRedirect to="workflows" />} />
                        <Route path="spine" element={<WorkspaceRedirect to="workflows" />} />
                        <Route path="issues" element={<WorkspaceRedirect to="workflows" />} />
                        {/* IssueDetailPage retired (#492): redirect to
                            the canonical artifact URL. */}
                        <Route
                          path="issues/:projectId/:number"
                          element={<IssueArtifactRedirect />}
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
                        {/* Cross-workflow runs list (#454 / ADR 0019).
                            Workspace-scoped; row clicks land on the flat
                            run detail route below. */}
                        <Route
                          path="runs"
                          element={<LazyRoute variant="list"><RunsPage /></LazyRoute>}
                        />
                        {/* Run detail hub (#303). Flat route — runs
                            have unique IDs (artifact_id), so they
                            don't need to be nested under workflow. */}
                        <Route
                          path="runs/:runId"
                          element={<LazyRoute variant="detail"><RunDetailPage /></LazyRoute>}
                        />
                        {/* Spec Plans surface (ADR 0023 / 0025, spec
                            #514). List of persisted plans, each with a
                            HitlCard-routed run button. */}
                        <Route
                          path="spec-plans"
                          element={<LazyRoute variant="list"><SpecPlansPage /></LazyRoute>}
                        />
                        {/* Guided "Create Plan" authoring surface
                            (spec #542) — sub-route, not a modal, for
                            the DAG preview + clean back-nav. */}
                        <Route
                          path="spec-plans/new"
                          element={<LazyRoute><CreateSpecPlanPage /></LazyRoute>}
                        />
                        {/* Spine event stream at operator grain
                            (#454 / ADR 0019). */}
                        <Route
                          path="activity"
                          element={<LazyRoute variant="list"><ActivityPage /></LazyRoute>}
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
