import { describe, it, expect, vi, beforeEach } from "vitest"
import { render, screen, waitFor } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { MemoryRouter, Routes, Route, useLocation } from "react-router-dom"

import { RunsPage } from "@/pages/RunsPage"
import { WorkspaceScope } from "@/lib/workspace"

// Runs-tab filter chips (#464 / ADR 0019). Pins:
// - the status + range chip rows render and round-trip through the URL,
// - the default time range is the last 7 days,
// - chip selections push status/since query params to the API client,
// - the workflow_id chip is read from the URL and clearable.

vi.mock("@/lib/auth", () => ({
  useAuth: () => ({ user: null, authEnabled: false }),
}))

const apiMock = vi.hoisted(() => ({
  listWorkspaces: vi.fn(),
  listWorkspaceRuns: vi.fn(),
  listWorkflows: vi.fn(),
}))

vi.mock("@/lib/api", async () => {
  const actual =
    await vi.importActual<typeof import("@/lib/api")>("@/lib/api")
  return { ...actual, api: apiMock }
})

const workspace = {
  id: "ws_1",
  slug: "acme",
  name: "Acme",
  created_by: "u1",
  created_at: "2026-01-01",
}

function LocationProbe() {
  const loc = useLocation()
  return (
    <div data-testid="probe-search">{loc.search}</div>
  )
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
                <>
                  <LocationProbe />
                  <Routes>
                    <Route index element={<RunsPage />} />
                    <Route path="runs" element={<RunsPage />} />
                  </Routes>
                </>
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
  apiMock.listWorkflows.mockResolvedValue({ workflows: [] })
  apiMock.listWorkspaceRuns.mockResolvedValue({ runs: [] })
})

describe("RunsPage filters (#464)", () => {
  it("defaults to 7-day range and sends `since` to the API", async () => {
    renderPage("/workspaces/acme/runs")
    await waitFor(() =>
      expect(apiMock.listWorkspaceRuns).toHaveBeenCalled(),
    )
    const lastCall = apiMock.listWorkspaceRuns.mock.calls.at(-1)!
    expect(lastCall[0]).toBe("ws_1")
    expect(lastCall[1].status).toBeUndefined()
    expect(typeof lastCall[1].since).toBe("string")
    const since = new Date(lastCall[1].since)
    // 7 days ± a 30s skew for test wall-clock drift.
    const ageMs = Date.now() - since.getTime()
    expect(ageMs).toBeGreaterThan(7 * 24 * 60 * 60 * 1000 - 30_000)
    expect(ageMs).toBeLessThan(7 * 24 * 60 * 60 * 1000 + 30_000)
    // Range chip row defaults to "Last 7d" pressed.
    const sevenDay = await screen.findByRole("button", { name: /Last 7d/ })
    expect(sevenDay.getAttribute("aria-pressed")).toBe("true")
  })

  it("clicking a status chip re-queries with status + updates URL", async () => {
    const user = userEvent.setup()
    renderPage("/workspaces/acme/runs")
    await waitFor(() =>
      expect(apiMock.listWorkspaceRuns).toHaveBeenCalled(),
    )
    const failedChip = await screen.findByRole("button", { name: "Failed" })
    await user.click(failedChip)
    await waitFor(() => {
      const last = apiMock.listWorkspaceRuns.mock.calls.at(-1)!
      expect(last[1].status).toBe("failed")
    })
    expect(screen.getByTestId("probe-search").textContent).toContain(
      "status=failed",
    )
  })

  it("clicking a range chip re-queries with the new `since` + updates URL", async () => {
    const user = userEvent.setup()
    renderPage("/workspaces/acme/runs")
    await waitFor(() =>
      expect(apiMock.listWorkspaceRuns).toHaveBeenCalled(),
    )
    const dayChip = await screen.findByRole("button", { name: /Last 24h/ })
    await user.click(dayChip)
    await waitFor(() => {
      const last = apiMock.listWorkspaceRuns.mock.calls.at(-1)!
      const since = new Date(last[1].since)
      const ageMs = Date.now() - since.getTime()
      expect(ageMs).toBeGreaterThan(24 * 60 * 60 * 1000 - 30_000)
      expect(ageMs).toBeLessThan(24 * 60 * 60 * 1000 + 30_000)
    })
    expect(screen.getByTestId("probe-search").textContent).toContain(
      "range=24h",
    )
  })

  it("range=all clears `since` and sends no time bound", async () => {
    const user = userEvent.setup()
    renderPage("/workspaces/acme/runs")
    await waitFor(() =>
      expect(apiMock.listWorkspaceRuns).toHaveBeenCalled(),
    )
    const allChips = await screen.findAllByRole("button", { name: "All" })
    // The status chip row also has an "All" — pick the one in the range row.
    const rangeAll = allChips.find((b) =>
      b.closest('[aria-label="Time range filter"]'),
    )
    expect(rangeAll).toBeTruthy()
    await user.click(rangeAll!)
    await waitFor(() => {
      const last = apiMock.listWorkspaceRuns.mock.calls.at(-1)!
      expect(last[1].since).toBeUndefined()
    })
    expect(screen.getByTestId("probe-search").textContent).toContain(
      "range=all",
    )
  })

  it("reads initial chip values from the URL", async () => {
    renderPage("/workspaces/acme/runs?status=passed&range=30d")
    await waitFor(() =>
      expect(apiMock.listWorkspaceRuns).toHaveBeenCalled(),
    )
    const last = apiMock.listWorkspaceRuns.mock.calls.at(-1)!
    expect(last[1].status).toBe("passed")
    const since = new Date(last[1].since)
    const ageMs = Date.now() - since.getTime()
    expect(ageMs).toBeGreaterThan(30 * 24 * 60 * 60 * 1000 - 30_000)
    expect(ageMs).toBeLessThan(30 * 24 * 60 * 60 * 1000 + 30_000)
    const passed = await screen.findByRole("button", { name: "Passed" })
    expect(passed.getAttribute("aria-pressed")).toBe("true")
    const thirtyDay = await screen.findByRole("button", { name: /Last 30d/ })
    expect(thirtyDay.getAttribute("aria-pressed")).toBe("true")
  })

  it("forwards workflow_id from the URL and shows a clearable chip", async () => {
    apiMock.listWorkflows.mockResolvedValue({
      workflows: [
        {
          id: "wf_abc",
          workspace_id: "ws_1",
          name: "My workflow",
          stages: [],
          active: false,
          status: "drafted",
        },
      ],
    })
    const user = userEvent.setup()
    renderPage("/workspaces/acme/runs?workflow_id=wf_abc")
    await waitFor(() => {
      const last = apiMock.listWorkspaceRuns.mock.calls.at(-1)!
      expect(last[1].workflowId).toBe("wf_abc")
    })
    const chip = await screen.findByRole("button", {
      name: /Clear workflow filter/i,
    })
    await user.click(chip)
    await waitFor(() => {
      const last = apiMock.listWorkspaceRuns.mock.calls.at(-1)!
      expect(last[1].workflowId).toBeUndefined()
    })
  })
})
