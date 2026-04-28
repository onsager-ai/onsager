import { describe, it, expect, vi, beforeEach } from "vitest"
import { render, screen, fireEvent, waitFor } from "@testing-library/react"
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { MemoryRouter, Routes, Route, useLocation } from "react-router-dom"

import { SidebarProvider } from "@/components/ui/sidebar"
import { WorkspaceSwitcher } from "@/components/workspaces/WorkspaceSwitcher"
import { WorkspaceScope } from "@/lib/workspace"

vi.mock("@/lib/auth", () => ({
  useAuth: () => ({ user: { id: "u1" }, authEnabled: true }),
}))

vi.mock("@/lib/api", () => ({
  api: {
    listWorkspaces: vi.fn().mockResolvedValue({
      workspaces: [
        {
          id: "w1",
          slug: "acme",
          name: "Acme",
          created_by: "u1",
          created_at: "2026-01-01",
        },
        {
          id: "w2",
          slug: "beta",
          name: "Beta",
          created_by: "u1",
          created_at: "2026-01-01",
        },
      ],
    }),
    createWorkspace: vi.fn(),
  },
}))

// Surface the current location in the DOM so tests can assert on
// post-switch navigation without bringing in a custom test harness.
function LocationProbe() {
  const loc = useLocation()
  return <div data-testid="location">{loc.pathname}</div>
}

function renderSwitcher(initialPath = "/workspaces/acme/sessions") {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return render(
    <QueryClientProvider client={qc}>
      <MemoryRouter initialEntries={[initialPath]}>
        <Routes>
          <Route
            path="/workspaces/:workspace/*"
            element={
              <WorkspaceScope>
                <SidebarProvider>
                  <WorkspaceSwitcher />
                  <LocationProbe />
                </SidebarProvider>
              </WorkspaceScope>
            }
          />
        </Routes>
      </MemoryRouter>
    </QueryClientProvider>,
  )
}

describe("WorkspaceSwitcher", () => {
  beforeEach(() => vi.clearAllMocks())

  it("renders the active workspace's name and slug", async () => {
    renderSwitcher()
    expect(await screen.findByText("Acme")).toBeInTheDocument()
    expect(screen.getByText("acme")).toBeInTheDocument()
  })

  it("opens the picker and lists every membership", async () => {
    renderSwitcher()
    fireEvent.click(
      await screen.findByRole("combobox", { name: /switch workspace/i }),
    )
    expect(await screen.findByText("Beta")).toBeInTheDocument()
    expect(screen.getByText(/create workspace/i)).toBeInTheDocument()
  })

  it(
    "navigates to the same resource segment under the picked workspace",
    async () => {
      renderSwitcher("/workspaces/acme/sessions")
      fireEvent.click(
        await screen.findByRole("combobox", { name: /switch workspace/i }),
      )
      fireEvent.click(await screen.findByText("Beta"))
      await waitFor(() =>
        expect(screen.getByTestId("location")).toHaveTextContent(
          "/workspaces/beta/sessions",
        ),
      )
    },
  )
})
