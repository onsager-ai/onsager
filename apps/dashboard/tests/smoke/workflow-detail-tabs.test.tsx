import { describe, it, expect, vi, beforeEach, afterEach } from "vitest"
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react"
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

// Lazy reveal + truncation contract (#466). The anchored sections
// truncate to "last N" with a `[show all]` control. Runs and
// Artifacts also lazy-load additional rows on scroll, modelled here
// by capturing the IntersectionObserver callback and firing it.
describe("WorkflowDetailPage lazy reveal (#466)", () => {
  const observers: Array<{
    cb: IntersectionObserverCallback
    targets: Element[]
  }> = []

  beforeEach(() => {
    vi.clearAllMocks()
    observers.length = 0
    window.history.replaceState(null, "", "/workspaces/acme/workflows/wf_1")

    class CaptureObserver {
      cb: IntersectionObserverCallback
      targets: Element[] = []
      constructor(cb: IntersectionObserverCallback) {
        this.cb = cb
        observers.push(this)
      }
      observe(el: Element) {
        this.targets.push(el)
      }
      unobserve() {}
      disconnect() {}
      takeRecords(): IntersectionObserverEntry[] {
        return []
      }
      readonly root = null
      readonly rootMargin = ""
      readonly thresholds: ReadonlyArray<number> = []
    }
    // @ts-expect-error replacing the stub installed by setup.ts
    globalThis.IntersectionObserver = CaptureObserver

    apiMock.listWorkspaces.mockResolvedValue({ workspaces: [workspace] })
    apiMock.getWorkflow.mockResolvedValue(workflow)
    apiMock.getWorkspaceWebhookDeliveriesHealth.mockResolvedValue({
      installations: [],
    })
    apiMock.getSpineEvents.mockResolvedValue({ events: [] })
    apiMock.listWorkflows.mockResolvedValue({
      workflows: [],
      counts: { drafted: 0, bound: 0, active: 0, paused: 0, all: 0 },
    })
    apiMock.getWorkflowVerdicts.mockResolvedValue({ verdicts: [] })
  })

  afterEach(() => {
    window.location.hash = ""
  })

  function makeRuns(n: number) {
    return Array.from({ length: n }, (_, i) => ({
      id: `run_${i}`,
      workflow_id: "wf_1",
      artifact_id: `art_${i}`,
      status: "passed" as const,
      stages: [{ stage_id: "s_1", status: "passed" as const }],
      started_at: new Date(2026, 4, 22, 12, 0, i).toISOString(),
      updated_at: new Date(2026, 4, 22, 12, 0, i).toISOString(),
    }))
  }

  function makeArtifacts(n: number) {
    return Array.from({ length: n }, (_, i) => ({
      id: `art_${i}`,
      kind: "Issue",
      name: `Artifact ${i}`,
      state: "drafted",
      owner: null,
      current_version: 0,
      consumers: {},
      external_ref: null,
      created_at: "2026-04-22T00:00:00Z",
      updated_at: "2026-04-22T00:00:00Z",
      last_observed_at: null,
    }))
  }

  it("Runs section truncates to last 10 and reveals more when the sentinel intersects", async () => {
    apiMock.getWorkflowRuns.mockResolvedValue({ runs: makeRuns(25) })
    apiMock.getWorkflowArtifacts.mockResolvedValue({ artifacts: [] })

    renderPage("/workspaces/acme/workflows/wf_1")

    await waitFor(() => {
      expect(screen.getByText("art_0")).toBeInTheDocument()
    })

    // First 10 visible, 11th not yet rendered.
    expect(screen.getByText("art_9")).toBeInTheDocument()
    expect(screen.queryByText("art_10")).toBeNull()

    // Header "Show all" link goes to the cross-workflow runs page.
    expect(
      screen.getByRole("link", { name: "Show all" }),
    ).toHaveAttribute(
      "href",
      "/workspaces/acme/runs?workflow_id=wf_1",
    )

    // Fire the sentinel observer → next chunk of 10 reveals.
    const obs = observers[0]
    expect(obs).toBeDefined()
    act(() => {
      obs.cb(
        obs.targets.map(
          (t) =>
            ({ isIntersecting: true, target: t }) as unknown as IntersectionObserverEntry,
        ),
        obs as unknown as IntersectionObserver,
      )
    })

    await waitFor(() => {
      expect(screen.getByText("art_19")).toBeInTheDocument()
    })
    expect(screen.queryByText("art_20")).toBeNull()
  })

  it("Artifacts section truncates to last 20 and expands on [Show all]", async () => {
    apiMock.getWorkflowRuns.mockResolvedValue({ runs: [] })
    apiMock.getWorkflowArtifacts.mockResolvedValue({
      artifacts: makeArtifacts(25),
    })

    renderPage("/workspaces/acme/workflows/wf_1")

    await waitFor(() => {
      expect(screen.getByText("Artifact 0")).toBeInTheDocument()
    })

    // First 20 rendered, 21st not yet.
    expect(screen.getByText("Artifact 19")).toBeInTheDocument()
    expect(screen.queryByText("Artifact 20")).toBeNull()

    const artifactsSection = document.getElementById("artifacts")!
    const showAll = within(artifactsSection).getByRole("button", {
      name: /Show all/,
    })
    expect(showAll).toHaveTextContent("Show all (5 more)")

    fireEvent.click(showAll)

    await waitFor(() => {
      expect(screen.getByText("Artifact 24")).toBeInTheDocument()
    })
    // Once everything is visible the trigger is gone.
    expect(
      within(artifactsSection).queryByRole("button", { name: /Show all/ }),
    ).toBeNull()
  })
})
