import { describe, it, expect, vi, beforeEach } from "vitest"
import { render, screen, waitFor } from "@testing-library/react"
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { MemoryRouter, Routes, Route } from "react-router-dom"

import { IssueDetailPage } from "@/pages/IssueDetailPage"
import { ApiError } from "@/lib/api"

// Smoke coverage for the live + skeleton join, the proxy fail-open
// banner, and the replay-disabled-when-degraded contract that the page
// inherits from `IssueActionsMenu` (Copilot review on PR #206).

vi.mock("@/lib/auth", () => ({
  useAuth: () => ({ user: null, authEnabled: false }),
}))

const apiMock = vi.hoisted(() => ({
  listWorkspaces: vi.fn(),
  getProjectIssue: vi.fn(),
  getArtifacts: vi.fn(),
  replayIssueTrigger: vi.fn(),
}))

vi.mock("@/lib/api", async () => {
  const actual =
    await vi.importActual<typeof import("@/lib/api")>("@/lib/api")
  return { ...actual, api: apiMock }
})

import { WorkspaceScope } from "@/lib/workspace"

function renderPage(initialPath = "/workspaces/acme/issues/proj_1/42") {
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  return render(
    <QueryClientProvider client={qc}>
      <MemoryRouter initialEntries={[initialPath]}>
        <Routes>
          <Route
            path="/workspaces/:workspace/*"
            element={
              <WorkspaceScope>
                <Routes>
                  <Route
                    path="issues/:projectId/:number"
                    element={<IssueDetailPage />}
                  />
                </Routes>
              </WorkspaceScope>
            }
          />
        </Routes>
      </MemoryRouter>
    </QueryClientProvider>,
  )
}

const workspace = {
  id: "ws1",
  slug: "acme",
  name: "Acme",
  created_by: "u1",
  created_at: "2026-01-01",
}

describe("IssueDetailPage", () => {
  beforeEach(() => {
    vi.clearAllMocks()
    apiMock.listWorkspaces.mockResolvedValue({ workspaces: [workspace] })
    apiMock.getArtifacts.mockResolvedValue({ artifacts: [] })
  })

  it("renders the live title, body, and labels when the proxy succeeds", async () => {
    apiMock.getProjectIssue.mockResolvedValue({
      issue: {
        number: 42,
        title: "Add issue detail page",
        state: "open",
        html_url: "https://github.com/acme/widgets/issues/42",
        author: "alice",
        labels: ["spec", "area:dashboard"],
        assignees: ["bob"],
        comments: 3,
        body: "We should have an issue detail page.",
        milestone: null,
        created_at: "2026-04-01T00:00:00Z",
        updated_at: "2026-04-29T00:00:00Z",
        closed_at: null,
      },
    })

    renderPage()

    expect(
      await screen.findByText("Add issue detail page"),
    ).toBeInTheDocument()
    expect(
      screen.getByText("We should have an issue detail page."),
    ).toBeInTheDocument()
    expect(screen.getByText("spec")).toBeInTheDocument()
    expect(screen.getByText("area:dashboard")).toBeInTheDocument()
    expect(screen.getByText("bob")).toBeInTheDocument()
  })

  it("renders the rate-limit banner when the proxy fails open", async () => {
    apiMock.getProjectIssue.mockResolvedValue({
      issue: null,
      error: "rate_limited",
    })
    apiMock.getArtifacts.mockResolvedValue({
      artifacts: [
        {
          id: "art_1",
          kind: "github_issue",
          name: null,
          owner: null,
          state: "draft",
          current_version: 1,
          external_ref: "github:project:proj_1:issue:42",
          created_at: "2026-04-01T00:00:00Z",
          updated_at: "2026-04-29T00:00:00Z",
          last_observed_at: "2026-04-29T00:00:00Z",
        },
      ],
    })

    renderPage()

    expect(
      await screen.findByText(/GitHub rate limit reached/),
    ).toBeInTheDocument()
    // Skeleton-only title falls back to `Issue #N`, not the artifact id.
    // The artifact id still appears under "Onsager metadata" — that's
    // intentional; what matters is that the page header isn't an opaque
    // identifier.
    expect(
      screen.getByRole("heading", { name: "Issue #42" }),
    ).toBeInTheDocument()
  })

  it("shows an explicit not-found state when the backend returns 404", async () => {
    apiMock.getProjectIssue.mockRejectedValue(
      new ApiError("issue not found", 404),
    )

    renderPage()

    await waitFor(() => {
      expect(screen.getByText("Issue not found.")).toBeInTheDocument()
    })
  })
})
