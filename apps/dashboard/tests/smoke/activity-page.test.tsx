import { describe, it, expect, vi, beforeEach } from "vitest"
import { render, screen, waitFor } from "@testing-library/react"
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { MemoryRouter, Routes, Route } from "react-router-dom"

import { ActivityPage } from "@/pages/ActivityPage"

// Activity tab — operator-grain spine event stream (ADR 0019 / spec #465).
// Pins:
// - The page calls the path-scoped `/api/workspaces/:id/activity`
//   endpoint (operator-grain is server-side; the page doesn't pass it).
// - Each row renders the event type + actor + timestamp + a deep-link
//   to the relevant artifact / run / workflow detail page.

vi.mock("@/lib/auth", () => ({
  useAuth: () => ({ user: null, authEnabled: false }),
}))

const apiMock = vi.hoisted(() => ({
  listWorkspaces: vi.fn(),
  getActivity: vi.fn(),
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

function renderPage() {
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  return render(
    <QueryClientProvider client={qc}>
      <MemoryRouter initialEntries={["/workspaces/acme/activity"]}>
        <Routes>
          <Route
            path="/workspaces/:workspace/*"
            element={
              <WorkspaceScope>
                <Routes>
                  <Route path="activity" element={<ActivityPage />} />
                </Routes>
              </WorkspaceScope>
            }
          />
        </Routes>
      </MemoryRouter>
    </QueryClientProvider>,
  )
}

beforeEach(() => {
  vi.clearAllMocks()
  apiMock.listWorkspaces.mockResolvedValue({ workspaces: [workspace] })
})

describe("ActivityPage (operator-grain feed, #465)", () => {
  it("calls the path-scoped operator-grain endpoint", async () => {
    apiMock.getActivity.mockResolvedValue({ events: [] })
    renderPage()
    await waitFor(() =>
      expect(apiMock.getActivity).toHaveBeenCalledWith("ws_1", { limit: 100 }),
    )
  })

  it("deep-links a run-keyed event row to /runs/:artifact_id", async () => {
    apiMock.getActivity.mockResolvedValue({
      events: [
        {
          id: 1,
          stream_id: "forge:art_42",
          stream_type: "substrate",
          event_type: "session.completed",
          data: { artifact_id: "art_42" },
          actor: "agent-node-1",
          created_at: "2026-05-23T03:00:00Z",
        },
      ],
    })
    renderPage()
    const link = await screen.findByRole("link", {
      name: /session\.completed/,
    })
    expect(link).toHaveAttribute("href", "/workspaces/acme/runs/art_42")
    // Actor + event_type are visible on the row.
    expect(link).toHaveTextContent("agent-node-1")
    expect(link).toHaveTextContent("session.completed")
  })

  it("deep-links a workflow-keyed event row to /workflows/:id", async () => {
    apiMock.getActivity.mockResolvedValue({
      events: [
        {
          id: 2,
          stream_id: "trigger:wf_3",
          stream_type: "substrate",
          event_type: "trigger.fired",
          data: { workflow_id: "wf_3" },
          actor: "system",
          created_at: "2026-05-23T03:01:00Z",
        },
      ],
    })
    renderPage()
    const link = await screen.findByRole("link", {
      name: /trigger\.fired/,
    })
    expect(link).toHaveAttribute("href", "/workspaces/acme/workflows/wf_3")
  })

  it("renders rows with no deep-link target as plain blocks", async () => {
    apiMock.getActivity.mockResolvedValue({
      events: [
        {
          id: 3,
          stream_id: "ising:health",
          stream_type: "ising",
          event_type: "ising.catchup_completed",
          data: {},
          actor: "ising-worker",
          created_at: "2026-05-23T03:02:00Z",
        },
      ],
    })
    renderPage()
    // The event type is rendered, but there's no link wrapping it.
    await waitFor(() =>
      expect(screen.getByText("ising.catchup_completed")).toBeInTheDocument(),
    )
    expect(screen.queryByRole("link")).toBeNull()
  })

  it("shows an empty state when no events come back", async () => {
    apiMock.getActivity.mockResolvedValue({ events: [] })
    renderPage()
    await waitFor(() =>
      expect(screen.getByText(/No activity/i)).toBeInTheDocument(),
    )
  })
})
