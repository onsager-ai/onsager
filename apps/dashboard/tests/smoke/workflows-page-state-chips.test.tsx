import { describe, it, expect, vi, beforeEach } from "vitest"
import { render, screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { MemoryRouter, Routes, Route } from "react-router-dom"

import { WorkflowsPage } from "@/pages/WorkflowsPage"
import { ChatUiProvider } from "@/lib/chat/use-chat-ui"

// Workflows-tab state filter chips + Drafted view (#467 / ADR 0019).
// Pins the chip row's count rendering, the URL `?state=` round-trip,
// and the Drafted view's surfaced affordances (carried over from the
// retired chat `DraftStrip`).

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

const COUNTS = {
  all: 6,
  drafted: 2,
  bound: 1,
  active: 2,
  paused: 1,
}

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

describe("WorkflowsPage state filter chips (#467)", () => {
  it("renders the chip row with counts from the endpoint", async () => {
    renderPage("/workspaces/acme/workflows")
    await waitFor(() =>
      expect(apiMock.listWorkflows).toHaveBeenCalledWith("ws_1", "all"),
    )
    const tablist = await screen.findByRole("tablist", {
      name: /workflow status filter/i,
    })
    expect(within(tablist).getByRole("tab", { name: /^All/ })).toBeTruthy()
    expect(within(tablist).getByLabelText("2 drafted")).toBeTruthy()
    expect(within(tablist).getByLabelText("1 bound")).toBeTruthy()
    expect(within(tablist).getByLabelText("2 active")).toBeTruthy()
    expect(within(tablist).getByLabelText("1 paused")).toBeTruthy()
  })

  it("clicking a chip re-queries with `state=<chip>`", async () => {
    const user = userEvent.setup()
    renderPage("/workspaces/acme/workflows")
    await waitFor(() =>
      expect(apiMock.listWorkflows).toHaveBeenCalledWith("ws_1", "all"),
    )
    const draftedChip = await screen.findByRole("tab", { name: /Drafted/ })
    await user.click(draftedChip)
    await waitFor(() =>
      expect(apiMock.listWorkflows).toHaveBeenCalledWith("ws_1", "drafted"),
    )
    // Once the click lands, the chip is marked selected — proxy for the
    // URL `?state=drafted` round-trip (MemoryRouter keeps location
    // in-memory, so the chip's aria-selected is the observable signal).
    await waitFor(() =>
      expect(
        screen.getByRole("tab", { name: /Drafted/ }).getAttribute("aria-selected"),
      ).toBe("true"),
    )
  })

  it("reads initial chip from the URL `?state=` query", async () => {
    renderPage("/workspaces/acme/workflows?state=active")
    await waitFor(() =>
      expect(apiMock.listWorkflows).toHaveBeenCalledWith("ws_1", "active"),
    )
    const activeChip = await screen.findByRole("tab", { name: /Active/ })
    expect(activeChip.getAttribute("aria-selected")).toBe("true")
  })

  it("falls back to `all` when `?state=` is unknown", async () => {
    renderPage("/workspaces/acme/workflows?state=nonsense")
    await waitFor(() =>
      expect(apiMock.listWorkflows).toHaveBeenCalledWith("ws_1", "all"),
    )
  })
})

describe("Drafted-chip view absorbs DraftStrip (#467)", () => {
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

  it("renders the local-drafts card only when the Drafted chip is active", async () => {
    seedDrafts([{ id: "d_1", name: "My draft" }])
    const user = userEvent.setup()
    renderPage("/workspaces/acme/workflows")
    await waitFor(() =>
      expect(apiMock.listWorkflows).toHaveBeenCalledWith("ws_1", "all"),
    )
    // `All` chip is the default — the local-drafts card is gated on the
    // Drafted chip being selected, so it should not appear here.
    expect(screen.queryByText("Local drafts")).toBeNull()
    const draftedChip = await screen.findByRole("tab", { name: /Drafted/ })
    await user.click(draftedChip)
    await screen.findByText("Local drafts")
    expect(screen.getByText("My draft")).toBeTruthy()
  })

  it("surfaces `Continue` and `New draft` affordances", async () => {
    seedDrafts([{ id: "d_1", name: "First draft" }])
    renderPage("/workspaces/acme/workflows?state=drafted")
    await screen.findByText("Local drafts")
    expect(screen.getAllByText("Continue").length).toBeGreaterThan(0)
    expect(screen.getByRole("button", { name: /New draft/i })).toBeTruthy()
  })

  it("shows the OSS `Drafts on this device.` footer when OSS with drafts", async () => {
    ossMock.useOSSFlag.mockReturnValue(true)
    seedDrafts([{ id: "d_1", name: "My draft" }])
    renderPage("/workspaces/acme/workflows?state=drafted")
    await screen.findByText("Local drafts")
    expect(screen.getByText("Drafts on this device.")).toBeTruthy()
  })
})
