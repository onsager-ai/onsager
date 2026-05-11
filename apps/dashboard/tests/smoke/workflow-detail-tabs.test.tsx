import { describe, it, expect, vi, beforeEach, afterEach } from "vitest"
import { render, screen, waitFor, fireEvent, act } from "@testing-library/react"
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { MemoryRouter, Routes, Route } from "react-router-dom"

import { WorkflowDetailPage } from "@/pages/WorkflowDetailPage"

// Smoke coverage for the four-tab hub refactor (#301 / PR 2a of #289):
// default tab is Runs, hash deep-links to a named tab, clicking a tab
// updates the URL hash, and a hashchange event from outside flips the
// active tab. The data-fetching tabs are mocked at the API surface; we
// only assert the navigation contract here.

vi.mock("@/lib/auth", () => ({
  useAuth: () => ({ user: null, authEnabled: false }),
}))

const apiMock = vi.hoisted(() => ({
  listWorkspaces: vi.fn(),
  getWorkflow: vi.fn(),
  getWorkflowRuns: vi.fn(),
  getArtifact: vi.fn(),
  getGovernanceEvents: vi.fn(),
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

describe("WorkflowDetailPage tabbed hub", () => {
  beforeEach(() => {
    vi.clearAllMocks()
    // Hash persistence is a real browser thing; reset between tests so a
    // stray `#artifacts` from one case doesn't leak into the next.
    window.history.replaceState(null, "", "/workspaces/acme/workflows/wf_1")
    apiMock.listWorkspaces.mockResolvedValue({ workspaces: [workspace] })
    apiMock.getWorkflow.mockResolvedValue(workflow)
    apiMock.getWorkflowRuns.mockResolvedValue({ runs: [] })
    apiMock.getArtifact.mockResolvedValue({
      artifact: {
        id: "a_1",
        kind: "Issue",
        name: "demo",
        owner: null,
        state: "active",
        current_version: 1,
        created_at: "",
        updated_at: "",
        created_by: "u",
        versions: [],
        vertical_lineage: [],
      },
    })
    apiMock.getGovernanceEvents.mockResolvedValue([])
    apiMock.getSpineEvents.mockResolvedValue({ events: [] })
    apiMock.listWorkflows.mockResolvedValue({ workflows: [] })
  })
  afterEach(() => {
    window.location.hash = ""
  })

  it("defaults to the Runs tab on first land", async () => {
    renderPage("/workspaces/acme/workflows/wf_1")
    const runsTab = await screen.findByRole("tab", { name: "Runs" })
    await waitFor(() =>
      expect(runsTab.getAttribute("data-active")).not.toBeNull(),
    )
    expect(
      screen.getByRole("tab", { name: "Definition" }).getAttribute("data-active"),
    ).toBeNull()
  })

  it("deep-links to the Artifacts tab via #artifacts in the URL", async () => {
    window.history.replaceState(
      null,
      "",
      "/workspaces/acme/workflows/wf_1#artifacts",
    )
    renderPage("/workspaces/acme/workflows/wf_1#artifacts")
    const artifactsTab = await screen.findByRole("tab", { name: "Artifacts" })
    await waitFor(() =>
      expect(artifactsTab.getAttribute("data-active")).not.toBeNull(),
    )
  })

  it("updates window.location.hash when a tab is clicked", async () => {
    renderPage("/workspaces/acme/workflows/wf_1")
    const definitionTab = await screen.findByRole("tab", { name: "Definition" })
    fireEvent.click(definitionTab)
    await waitFor(() =>
      expect(definitionTab.getAttribute("data-active")).not.toBeNull(),
    )
    expect(window.location.hash).toBe("#definition")
  })

  it("reacts to an external hashchange (browser back/forward)", async () => {
    renderPage("/workspaces/acme/workflows/wf_1")
    await screen.findByRole("tab", { name: "Verdicts" })
    act(() => {
      window.history.replaceState(
        null,
        "",
        "/workspaces/acme/workflows/wf_1#verdicts",
      )
      window.dispatchEvent(new HashChangeEvent("hashchange"))
    })
    await waitFor(() =>
      expect(
        screen
          .getByRole("tab", { name: "Verdicts" })
          .getAttribute("data-active"),
      ).not.toBeNull(),
    )
  })
})
