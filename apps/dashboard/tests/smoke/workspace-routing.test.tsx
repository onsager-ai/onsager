import { describe, it, expect } from "vitest"
import { render, screen, waitFor } from "@testing-library/react"
import {
  MemoryRouter,
  Routes,
  Route,
  Navigate,
  useLocation,
  useParams,
} from "react-router-dom"

// ADR 0019 redirect contract. Replaces the legacy spec #398 `/` → `/chat`
// row. After ADR 0019:
//
// - `/` redirects to the user's first workspace's Workflows tab (or to
//   the workspace picker when there are zero memberships).
// - `/chat` is retired as a page route (#457) and redirects to the same
//   workspace overview as the bare path.
// - `/workspaces/:slug/chat` redirects to `/workspaces/:slug/workflows`.
//
// We replicate the App.tsx redirect rows here rather than rendering the
// full App so the test stays light (no AuthProvider, no QueryClient).

function BarePathRedirect({ slug }: { slug?: string }) {
  return (
    <Navigate
      to={slug ? `/workspaces/${slug}/workflows` : "/workspaces"}
      replace
    />
  )
}

function WorkspaceRedirect({ to }: { to: string }) {
  const { workspace } = useParams<{ workspace: string }>()
  return <Navigate to={`/workspaces/${workspace}/${to}`} replace />
}

function LocationProbe() {
  const loc = useLocation()
  return <div data-testid="location">{loc.pathname}</div>
}

function renderRoute(initial: string, slug = "acme") {
  return render(
    <MemoryRouter initialEntries={[initial]}>
      <Routes>
        <Route path="/" element={<BarePathRedirect slug={slug} />} />
        <Route path="/chat" element={<BarePathRedirect slug={slug} />} />
        <Route path="/workspaces" element={<LocationProbe />} />
        <Route
          path="/workspaces/:workspace/chat"
          element={<WorkspaceRedirect to="workflows" />}
        />
        <Route path="/workspaces/:workspace/*" element={<LocationProbe />} />
      </Routes>
    </MemoryRouter>,
  )
}

describe("ADR 0019 redirect contract (#453, #457)", () => {
  it("bare `/` lands on the user's first workspace's Workflows tab", async () => {
    renderRoute("/")
    await waitFor(() =>
      expect(screen.getByTestId("location")).toHaveTextContent(
        "/workspaces/acme/workflows",
      ),
    )
  })

  it("bare `/` falls back to the workspace picker when no workspace exists", async () => {
    render(
      <MemoryRouter initialEntries={["/"]}>
        <Routes>
          <Route path="/" element={<BarePathRedirect />} />
          <Route path="/workspaces" element={<LocationProbe />} />
        </Routes>
      </MemoryRouter>,
    )
    await waitFor(() =>
      expect(screen.getByTestId("location")).toHaveTextContent("/workspaces"),
    )
  })

  it("`/chat` redirects to the user's workspace overview (#457)", async () => {
    renderRoute("/chat")
    await waitFor(() =>
      expect(screen.getByTestId("location")).toHaveTextContent(
        "/workspaces/acme/workflows",
      ),
    )
  })

  it("`/workspaces/:slug/chat` redirects to `/workspaces/:slug/workflows` (#457)", async () => {
    renderRoute("/workspaces/foo/chat", "foo")
    await waitFor(() =>
      expect(screen.getByTestId("location")).toHaveTextContent(
        "/workspaces/foo/workflows",
      ),
    )
  })
})
