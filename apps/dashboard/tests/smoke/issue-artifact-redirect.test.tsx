import { describe, it, expect, vi, beforeEach } from "vitest"
import { render, screen, waitFor } from "@testing-library/react"
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import {
  MemoryRouter,
  Routes,
  Route,
  useLocation,
} from "react-router-dom"

// ADR 0021 / #492: IssueDetailPage is retired. `/issues/:projectId/:number`
// resolves to the canonical `/artifacts/:id` for the matching
// `github_issue` artifact via the same `external_ref` join the page used.

vi.mock("@/lib/auth", () => ({
  useAuth: () => ({ user: null, authEnabled: false }),
}))

const apiMock = vi.hoisted(() => ({
  listWorkspaces: vi.fn(),
  getArtifacts: vi.fn(),
}))

vi.mock("@/lib/api", async () => {
  const actual =
    await vi.importActual<typeof import("@/lib/api")>("@/lib/api")
  return { ...actual, api: apiMock }
})

import { IssueArtifactRedirect } from "@/App"
import { WorkspaceScope } from "@/lib/workspace"

const workspace = {
  id: "ws_1",
  slug: "acme",
  name: "Acme",
  created_by: "u1",
  created_at: "2026-01-01",
}

function artifact(overrides: Record<string, unknown>) {
  return {
    id: "art_1",
    kind: "github_issue",
    name: null,
    state: "draft",
    owner: null,
    current_version: 1,
    consumers: [],
    external_ref: "github:project:proj_1:issue:42",
    created_at: "2026-04-01T00:00:00Z",
    updated_at: "2026-04-29T00:00:00Z",
    last_observed_at: null,
    ...overrides,
  }
}

function LocationProbe() {
  const loc = useLocation()
  return <div data-testid="location">{loc.pathname}</div>
}

function renderRedirect(initial: string) {
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  return render(
    <QueryClientProvider client={qc}>
      <MemoryRouter initialEntries={[initial]}>
        <Routes>
          <Route
            path="/workspaces/:workspace/*"
            element={
              <WorkspaceScope>
                <Routes>
                  <Route
                    path="issues/:projectId/:number"
                    element={<IssueArtifactRedirect />}
                  />
                  <Route path="artifacts/:id" element={<LocationProbe />} />
                  <Route path="workflows" element={<LocationProbe />} />
                </Routes>
              </WorkspaceScope>
            }
          />
        </Routes>
      </MemoryRouter>
    </QueryClientProvider>,
  )
}

describe("IssueArtifactRedirect (#492)", () => {
  beforeEach(() => {
    vi.clearAllMocks()
    apiMock.listWorkspaces.mockResolvedValue({ workspaces: [workspace] })
  })

  it("redirects to the matching artifact's canonical URL", async () => {
    apiMock.getArtifacts.mockResolvedValue({
      artifacts: [artifact({ id: "art_42" })],
    })
    renderRedirect("/workspaces/acme/issues/proj_1/42")
    await waitFor(() =>
      expect(screen.getByTestId("location")).toHaveTextContent(
        "/workspaces/acme/artifacts/art_42",
      ),
    )
    expect(apiMock.getArtifacts).toHaveBeenCalledWith("ws_1", {
      kind: "github_issue",
      project_id: "proj_1",
    })
  })

  it("falls back to Workflows when no artifact matches (un-ingested issue)", async () => {
    apiMock.getArtifacts.mockResolvedValue({ artifacts: [] })
    renderRedirect("/workspaces/acme/issues/proj_1/999")
    await waitFor(() =>
      expect(screen.getByTestId("location")).toHaveTextContent(
        "/workspaces/acme/workflows",
      ),
    )
  })

  it("falls back to Workflows on a malformed issue number", async () => {
    renderRedirect("/workspaces/acme/issues/proj_1/42abc")
    await waitFor(() =>
      expect(screen.getByTestId("location")).toHaveTextContent(
        "/workspaces/acme/workflows",
      ),
    )
    expect(apiMock.getArtifacts).not.toHaveBeenCalled()
  })
})
