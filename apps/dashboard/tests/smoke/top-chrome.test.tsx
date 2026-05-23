import { describe, it, expect, vi, beforeEach } from "vitest"
import {
  render,
  screen,
  fireEvent,
  waitFor,
  within,
} from "@testing-library/react"
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { MemoryRouter, Route, Routes, useLocation } from "react-router-dom"

import { AppLayout } from "@/components/layout/AppLayout"

// Top chrome shell from #463 / ADR 0019. Two Test items the spec
// pins are covered here:
//
//  - Component test: the top chrome renders all regions at desktop AND
//    mobile widths. jsdom doesn't apply media queries, so both the
//    desktop chrome (`<nav aria-label="Workspace sections">`) and the
//    mobile tabs row (`<nav aria-label="Workspace sections (mobile)">`)
//    are present in the DOM; we assert each region exposes the same
//    three tabs and that the desktop chrome carries the logo, workspace
//    switcher, ⌘K trigger, and user-menu button.
//  - Tab switching preserves workspace scope across the three tabs:
//    clicking each tab inside either nav navigates to
//    `/workspaces/<slug>/<tab>`, never bare `/<tab>`.

vi.mock("@/lib/auth", () => ({
  useAuth: () => ({
    user: {
      id: "u1",
      github_login: "alice",
      github_name: "Alice",
      github_avatar_url: null,
    },
    logout: vi.fn(),
    authEnabled: true,
  }),
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
      ],
    }),
    createWorkspace: vi.fn(),
  },
}))

function LocationProbe() {
  const loc = useLocation()
  return <div data-testid="location">{loc.pathname}</div>
}

function renderChrome(initialPath = "/workspaces/acme/workflows") {
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
              <AppLayout>
                <LocationProbe />
              </AppLayout>
            }
          />
        </Routes>
      </MemoryRouter>
    </QueryClientProvider>,
  )
}

describe("top chrome (#463 / ADR 0019)", () => {
  beforeEach(() => vi.clearAllMocks())

  it("renders the desktop chrome with logo, switcher, three tabs, ⌘K, and avatar", async () => {
    renderChrome()
    // Wait for the memberships query to resolve so the chrome's
    // workspace-scoped hrefs are populated; before then the fallback
    // target is `/workspaces`.
    expect(await screen.findByText("Acme")).toBeInTheDocument()
    // Logo points at the active workspace's Workflows tab.
    const home = screen.getByRole("link", { name: /onsager home/i })
    expect(home).toHaveAttribute("href", "/workspaces/acme/workflows")
    // Desktop nav region exists and lists all three tabs as links.
    const desktopNav = screen.getByRole("navigation", {
      name: /workspace sections$/i,
    })
    for (const tab of ["Workflows", "Runs", "Activity"]) {
      const link = within(desktopNav).getByRole("link", { name: tab })
      expect(link).toHaveAttribute("href", `/workspaces/acme/${tab.toLowerCase()}`)
    }
    // ⌘K and avatar/user-menu triggers live in the right cluster.
    expect(
      screen.getAllByRole("button", { name: /open command palette/i }).length,
    ).toBeGreaterThan(0)
    expect(
      screen.getAllByRole("button", { name: /open user menu/i }).length,
    ).toBeGreaterThan(0)
  })

  it("renders the mobile chrome with a second tabs row carrying all three tabs", async () => {
    renderChrome()
    expect(await screen.findByText("Acme")).toBeInTheDocument()
    const mobileNav = screen.getByRole("navigation", {
      name: /workspace sections \(mobile\)/i,
    })
    for (const tab of ["Workflows", "Runs", "Activity"]) {
      const link = within(mobileNav).getByRole("link", { name: tab })
      expect(link).toHaveAttribute("href", `/workspaces/acme/${tab.toLowerCase()}`)
    }
  })

  it("tab switching preserves workspace scope across all three tabs", async () => {
    renderChrome("/workspaces/acme/workflows")
    // Wait for the active workspace to resolve so tab hrefs are scoped
    // (without it the fallback target is `/workspaces`).
    await screen.findByText("Acme")
    const desktopNav = screen.getByRole("navigation", {
      name: /workspace sections$/i,
    })

    fireEvent.click(within(desktopNav).getByRole("link", { name: "Runs" }))
    await waitFor(() =>
      expect(screen.getByTestId("location")).toHaveTextContent(
        "/workspaces/acme/runs",
      ),
    )

    fireEvent.click(within(desktopNav).getByRole("link", { name: "Activity" }))
    await waitFor(() =>
      expect(screen.getByTestId("location")).toHaveTextContent(
        "/workspaces/acme/activity",
      ),
    )

    fireEvent.click(within(desktopNav).getByRole("link", { name: "Workflows" }))
    await waitFor(() =>
      expect(screen.getByTestId("location")).toHaveTextContent(
        "/workspaces/acme/workflows",
      ),
    )
  })
})
