import { describe, it, expect, vi, beforeEach, afterEach } from "vitest"
import { render, screen, waitFor } from "@testing-library/react"
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { MemoryRouter, Routes, Route } from "react-router-dom"

import { WorkflowDetailPage } from "@/pages/WorkflowDetailPage"

// Anchored timeline contract (#455 / ADR 0019). The legacy four-tab
// shell was collapsed to a single scroll surface with four anchored
// sections. Hash anchors (`#definition`, `#runs`, `#artifacts`,
// `#verdicts`) are preserved as scroll targets so bookmarks survive.

vi.mock("@/lib/auth", () => ({
  useAuth: () => ({ user: null, authEnabled: false }),
}))

const apiMock = vi.hoisted(() => ({
  listWorkspaces: vi.fn(),
  getWorkflow: vi.fn(),
  getWorkflowRuns: vi.fn(),
  getWorkflowArtifacts: vi.fn(),
  getWorkflowVerdicts: vi.fn(),
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
        artifact_kind: "Issue",
        config: {},
      },
    ],
    created_at: "2026-04-22T00:00:00Z",
    updated_at: "2026-04-22T00:00:00Z",
  },
}

function renderPage(initialPath: string) {
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
                    path="workflows/:id"
                    element={<WorkflowDetailPage />}
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

describe("WorkflowDetailPage anchored timeline (#455)", () => {
  beforeEach(() => {
    vi.clearAllMocks()
    window.history.replaceState(null, "", "/workspaces/acme/workflows/wf_1")
    apiMock.listWorkspaces.mockResolvedValue({ workspaces: [workspace] })
    apiMock.getWorkflow.mockResolvedValue(workflow)
    apiMock.getWorkflowRuns.mockResolvedValue({ runs: [] })
    apiMock.getWorkflowArtifacts.mockResolvedValue({ artifacts: [] })
    apiMock.getWorkflowVerdicts.mockResolvedValue({ verdicts: [] })
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
    window.location.hash = ""
  })

  it("renders all four anchored sections with stable ids", async () => {
    renderPage("/workspaces/acme/workflows/wf_1")
    await waitFor(() => {
      expect(
        screen.getByRole("heading", { name: "Definition" }),
      ).toBeInTheDocument()
      expect(
        screen.getByRole("heading", { name: "Runs" }),
      ).toBeInTheDocument()
      expect(
        screen.getByRole("heading", { name: "Artifacts" }),
      ).toBeInTheDocument()
      expect(
        screen.getByRole("heading", { name: "Verdicts" }),
      ).toBeInTheDocument()
    })
    expect(document.getElementById("definition")).not.toBeNull()
    expect(document.getElementById("runs")).not.toBeNull()
    expect(document.getElementById("artifacts")).not.toBeNull()
    expect(document.getElementById("verdicts")).not.toBeNull()
  })

  it("no longer renders the legacy tab UI", async () => {
    renderPage("/workspaces/acme/workflows/wf_1#runs")
    await waitFor(() =>
      expect(
        screen.getByRole("heading", { name: "Runs" }),
      ).toBeInTheDocument(),
    )
    expect(screen.queryByRole("tab", { name: "Definition" })).toBeNull()
    expect(screen.queryByRole("tab", { name: "Runs" })).toBeNull()
    expect(screen.queryByRole("tab", { name: "Artifacts" })).toBeNull()
    expect(screen.queryByRole("tab", { name: "Verdicts" })).toBeNull()
  })
})
