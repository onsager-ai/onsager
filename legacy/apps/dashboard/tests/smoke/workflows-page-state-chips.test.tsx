import { describe, it, expect, vi, beforeEach } from "vitest"
import { render, screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { MemoryRouter, Routes, Route } from "react-router-dom"

import { WorkflowsPage } from "@/pages/WorkflowsPage"
import { ChatUiProvider } from "@/lib/chat/use-chat-ui"

// Workflows-tab two-axis filter chips + Drafted view (#467 / ADR 0019,
// migrated to the readiness/autofire axes by spec #509). Pins the per-axis
// count rendering, the URL `?readiness=`/`?autofire=` round-trip, and the
// Drafted view's surfaced affordances (carried over from the retired chat
// `DraftStrip`).

vi.mock("@/lib/auth", () => ({
  useAuth: () => ({ user: null, authEnabled: false }),
}))

const apiMock = vi.hoisted(() => ({
  listWorkspaces: vi.fn(),
  listWorkflows: vi.fn(),
  getWorkspaceWebhookDeliveriesHealth: vi.fn(),
  getWorkflowRuns: vi.fn(),
}))

vi.mock("@/lib/api", async () => {
  const actual =
    await vi.importActual<typeof import("@/lib/api")>("@/lib/api")
  return { ...actual, api: apiMock }
})

const ossMock = vi.hoisted(() => ({ useOSSFlag: vi.fn() }))
vi.mock("@/hooks/useOSSFlag", () => ossMock)

import { WorkspaceScope } from "@/lib/workspace"

const workspace = {
  id: "ws_1",
  slug: "acme",
  name: "Acme",
  created_by: "u1",
  created_at: "2026-01-01",
}

function renderPage(initialPath: string) {
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  return render(
    <QueryClientProvider client={qc}>
      <MemoryRouter initialEntries={[initialPath]}>
        <ChatUiProvider>
          <Routes>
            <Route
              path="/workspaces/:workspace/*"
              element={
                <WorkspaceScope>
                  <Routes>
                    <Route index element={<WorkflowsPage />} />
                    <Route path="workflows" element={<WorkflowsPage />} />
                  </Routes>
                </WorkspaceScope>
              }
            />
          </Routes>
        </ChatUiProvider>
      </MemoryRouter>
    </QueryClientProvider>,
  )
}

// Per-axis counts shape (spec #509): each axis reports its own buckets.
const COUNTS = {
  all: 6,
  readiness: { drafted: 2, ready: 4 },
  autofire: { off: 1, active: 2, paused: 1 },
}

// The page calls `listWorkflows(workspaceId, { readiness, autofire })`.
const ALL_FILTER = { readiness: "all", autofire: "all" }

beforeEach(() => {
  vi.clearAllMocks()
  window.localStorage.clear()
  ossMock.useOSSFlag.mockReturnValue(false)
  apiMock.listWorkspaces.mockResolvedValue({ workspaces: [workspace] })
  apiMock.getWorkspaceWebhookDeliveriesHealth.mockResolvedValue({
    installations: [],
  })
  apiMock.getWorkflowRuns.mockResolvedValue({ runs: [] })
  apiMock.listWorkflows.mockResolvedValue({
    workflows: [],
    counts: COUNTS,
  })
})

describe("WorkflowsPage two-axis filter chips (#467 / #509)", () => {
  it("renders both axis groups with per-axis counts from the endpoint", async () => {
    renderPage("/workspaces/acme/workflows")
    await waitFor(() =>
      expect(apiMock.listWorkflows).toHaveBeenCalledWith("ws_1", ALL_FILTER),
    )
    const readiness = await screen.findByRole("tablist", {
      name: /workflow readiness filter/i,
    })
    const autofire = await screen.findByRole("tablist", {
      name: /workflow autofire filter/i,
    })
    // readiness group: All / Drafted / Ready
    expect(within(readiness).getByLabelText("2 drafted")).toBeTruthy()
    expect(within(readiness).getByLabelText("4 ready")).toBeTruthy()
    // autofire group: All / Manual / Active / Paused
    expect(within(autofire).getByLabelText("1 manual")).toBeTruthy()
    expect(within(autofire).getByLabelText("2 active")).toBeTruthy()
    expect(within(autofire).getByLabelText("1 paused")).toBeTruthy()
    // Each group has its own "All" chip.
    expect(within(readiness).getByRole("tab", { name: /^All/ })).toBeTruthy()
    expect(within(autofire).getByRole("tab", { name: /^All/ })).toBeTruthy()
  })

  it("clicking the Drafted readiness chip re-queries with readiness=drafted", async () => {
    const user = userEvent.setup()
    renderPage("/workspaces/acme/workflows")
    await waitFor(() =>
      expect(apiMock.listWorkflows).toHaveBeenCalledWith("ws_1", ALL_FILTER),
    )
    const readiness = await screen.findByRole("tablist", {
      name: /workflow readiness filter/i,
    })
    await user.click(within(readiness).getByRole("tab", { name: /Drafted/ }))
    await waitFor(() =>
      expect(apiMock.listWorkflows).toHaveBeenCalledWith("ws_1", {
        readiness: "drafted",
        autofire: "all",
      }),
    )
    await waitFor(() =>
      expect(
        within(readiness)
          .getByRole("tab", { name: /Drafted/ })
          .getAttribute("aria-selected"),
      ).toBe("true"),
    )
  })

  it("clicking the Active autofire chip re-queries with autofire=active", async () => {
    const user = userEvent.setup()
    renderPage("/workspaces/acme/workflows")
    await waitFor(() =>
      expect(apiMock.listWorkflows).toHaveBeenCalledWith("ws_1", ALL_FILTER),
    )
    const autofire = await screen.findByRole("tablist", {
      name: /workflow autofire filter/i,
    })
    await user.click(within(autofire).getByRole("tab", { name: /Active/ }))
    await waitFor(() =>
      expect(apiMock.listWorkflows).toHaveBeenCalledWith("ws_1", {
        readiness: "all",
        autofire: "active",
      }),
    )
  })

  it("reads initial axis values from the URL query", async () => {
    renderPage("/workspaces/acme/workflows?readiness=ready&autofire=paused")
    await waitFor(() =>
      expect(apiMock.listWorkflows).toHaveBeenCalledWith("ws_1", {
        readiness: "ready",
        autofire: "paused",
      }),
    )
    const readiness = await screen.findByRole("tablist", {
      name: /workflow readiness filter/i,
    })
    expect(
      within(readiness)
        .getByRole("tab", { name: /Ready/ })
        .getAttribute("aria-selected"),
    ).toBe("true")
  })

  it("falls back to `all` when a URL axis value is unknown", async () => {
    renderPage("/workspaces/acme/workflows?readiness=nonsense")
    await waitFor(() =>
      expect(apiMock.listWorkflows).toHaveBeenCalledWith("ws_1", ALL_FILTER),
    )
  })
})

describe("Drafted view absorbs DraftStrip (#467)", () => {
  function seedDrafts(items: { id: string; name: string }[]) {
    const drafts = items.map((it) => ({
      id: it.id,
      user_id: "anon",
      name: it.name,
      source: "blank" as const,
      workflow: {
        name: "",
        trigger: { install_id: "", repo_owner: "", repo_name: "", label: "" },
        stages: [],
      },
      created_at: new Date().toISOString(),
      updated_at: new Date().toISOString(),
    }))
    window.localStorage.setItem(
      "onsager.drafts.anon",
      JSON.stringify({ drafts }),
    )
  }

  it("renders the local-drafts card only when the Drafted readiness chip is active", async () => {
    seedDrafts([{ id: "d_1", name: "My draft" }])
    const user = userEvent.setup()
    renderPage("/workspaces/acme/workflows")
    await waitFor(() =>
      expect(apiMock.listWorkflows).toHaveBeenCalledWith("ws_1", ALL_FILTER),
    )
    // Default `All` readiness — the local-drafts card is gated on the
    // Drafted readiness chip being selected, so it should not appear here.
    expect(screen.queryByText("Local drafts")).toBeNull()
    const readiness = await screen.findByRole("tablist", {
      name: /workflow readiness filter/i,
    })
    await user.click(within(readiness).getByRole("tab", { name: /Drafted/ }))
    await screen.findByText("Local drafts")
    expect(screen.getByText("My draft")).toBeTruthy()
  })

  it("surfaces `Continue` and `New draft` affordances", async () => {
    seedDrafts([{ id: "d_1", name: "First draft" }])
    renderPage("/workspaces/acme/workflows?readiness=drafted")
    await screen.findByText("Local drafts")
    expect(screen.getAllByText("Continue").length).toBeGreaterThan(0)
    expect(screen.getByRole("button", { name: /New draft/i })).toBeTruthy()
  })

  it("shows the OSS `Drafts on this device.` footer when OSS with drafts", async () => {
    ossMock.useOSSFlag.mockReturnValue(true)
    seedDrafts([{ id: "d_1", name: "My draft" }])
    renderPage("/workspaces/acme/workflows?readiness=drafted")
    await screen.findByText("Local drafts")
    expect(screen.getByText("Drafts on this device.")).toBeTruthy()
  })
})
