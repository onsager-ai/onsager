import { describe, it, expect, vi, beforeEach, afterEach } from "vitest"
import { render, screen, waitFor } from "@testing-library/react"
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import {
  MemoryRouter,
  Routes,
  Route,
  useLocation,
} from "react-router-dom"

import { WorkflowDetailPage } from "@/pages/WorkflowDetailPage"

// ADR 0021 / #490 contract: WorkflowDetailPage collapses from four
// sections (Definition / Runs / Artifacts / Verdicts) to two —
// Definition + Activity. `#runs` scrolls to Activity; `#artifacts` and
// `#verdicts` 301-redirect to the workflow-filtered Activity tab so
// bookmarks against the retired sections still resolve.

vi.mock("@/lib/auth", () => ({
  useAuth: () => ({ user: null, authEnabled: false }),
}))

// The header ChatAskButton calls useChatUi; stub it so the page renders
// without the full ChatUiProvider tree (chat is exercised separately in
// chat-ask-button.test.tsx).
vi.mock("@/lib/chat/use-chat-ui", () => ({
  useChatUi: () => ({ open: () => {} }),
}))

const apiMock = vi.hoisted(() => ({
  listWorkspaces: vi.fn(),
  getWorkflow: vi.fn(),
  getWorkflowRuns: vi.fn(),
  getWorkspaceWebhookDeliveriesHealth: vi.fn(),
  getSpineEvents: vi.fn(),
  listWorkflows: vi.fn(),
}))

vi.mock("@/lib/api", async () => {
  const actual =
    await vi.importActual<typeof import("@/lib/api")>("@/lib/api")
  return { ...actual, api: apiMock }
})

import { WorkspaceScope } from "@/lib/workspace"

const workspace = {
  id: "ws_1",
  slug: "acme",
  name: "Acme",
  created_by: "u1",
  created_at: "2026-01-01",
}

const workflow = {
  workflow: {
    id: "wf_1",
    workspace_id: "ws_1",
    name: "Issue → PR",
    preset: null,
    status: "active",
    trigger: {
      kind: "github-label",
      install_id: "42",
      repo_owner: "onsager-ai",
      repo_name: "onsager",
      label: "factory",
      kind_tag: "github_issue_webhook",
    },
    stages: [
      {
        id: "s_1",
        name: "Triage",
        gate_kind: "agent-session",
        config: {},
      },
    ],
    created_at: "2026-04-22T00:00:00Z",
    updated_at: "2026-04-22T00:00:00Z",
  },
}

function LocationProbe() {
  const loc = useLocation()
  return (
    <div data-testid="location">
      {loc.pathname}
      {loc.search}
    </div>
  )
}

function renderPage(initialPath = "/workspaces/acme/workflows/wf_1") {
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
                  <Route path="workflows/:id" element={<WorkflowDetailPage />} />
                  <Route path="activity" element={<LocationProbe />} />
                </Routes>
              </WorkspaceScope>
            }
          />
        </Routes>
      </MemoryRouter>
    </QueryClientProvider>,
  )
}

describe("WorkflowDetailPage sections (ADR 0021 / #490)", () => {
  beforeEach(() => {
    vi.clearAllMocks()
    window.history.replaceState(null, "", "/workspaces/acme/workflows/wf_1")
    apiMock.listWorkspaces.mockResolvedValue({ workspaces: [workspace] })
    apiMock.getWorkflow.mockResolvedValue(workflow)
    apiMock.getWorkflowRuns.mockResolvedValue({ runs: [] })
    apiMock.getWorkspaceWebhookDeliveriesHealth.mockResolvedValue({
      installations: [],
    })
    apiMock.getSpineEvents.mockResolvedValue({ events: [] })
    apiMock.listWorkflows.mockResolvedValue({
      workflows: [],
      counts: { drafted: 0, bound: 0, active: 0, paused: 0, all: 0 },
    })
  })
  afterEach(() => {
    window.history.replaceState(null, "", "/workspaces/acme/workflows/wf_1")
  })

  it("renders exactly two sections — Definition + Activity", async () => {
    renderPage()
    await waitFor(() => {
      expect(
        screen.getByRole("heading", { name: "Definition" }),
      ).toBeInTheDocument()
      expect(
        screen.getByRole("heading", { name: "Activity" }),
      ).toBeInTheDocument()
    })
    expect(document.getElementById("definition")).not.toBeNull()
    expect(document.getElementById("activity")).not.toBeNull()

    // Retired sections are gone.
    expect(screen.queryByRole("heading", { name: "Runs" })).toBeNull()
    expect(screen.queryByRole("heading", { name: "Artifacts" })).toBeNull()
    expect(screen.queryByRole("heading", { name: "Verdicts" })).toBeNull()
    expect(document.getElementById("runs")).toBeNull()
    expect(document.getElementById("artifacts")).toBeNull()
    expect(document.getElementById("verdicts")).toBeNull()
  })

  it("uses no tab UI as primary IA", async () => {
    renderPage()
    await waitFor(() =>
      expect(
        screen.getByRole("heading", { name: "Activity" }),
      ).toBeInTheDocument(),
    )
    expect(screen.queryByRole("tab")).toBeNull()
  })

  it("links Activity out to the workflow-filtered Activity tab", async () => {
    renderPage()
    const link = await screen.findByRole("link", {
      name: "View all in Activity",
    })
    expect(link).toHaveAttribute(
      "href",
      "/workspaces/acme/activity?workflow=wf_1",
    )
  })

  it("`#artifacts` redirects to the workflow-filtered Activity tab", async () => {
    window.history.replaceState(
      null,
      "",
      "/workspaces/acme/workflows/wf_1#artifacts",
    )
    renderPage("/workspaces/acme/workflows/wf_1#artifacts")
    await waitFor(() => {
      const loc = screen.getByTestId("location")
      expect(loc).toHaveTextContent("/workspaces/acme/activity")
      expect(loc).toHaveTextContent("workflow=wf_1")
      expect(loc).toHaveTextContent("source_type=artifact")
    })
  })

  it("`#verdicts` redirects to the workflow-filtered Activity tab", async () => {
    window.history.replaceState(
      null,
      "",
      "/workspaces/acme/workflows/wf_1#verdicts",
    )
    renderPage("/workspaces/acme/workflows/wf_1#verdicts")
    await waitFor(() => {
      const loc = screen.getByTestId("location")
      expect(loc).toHaveTextContent("/workspaces/acme/activity")
      expect(loc).toHaveTextContent("source_type=verdict")
    })
  })
})
