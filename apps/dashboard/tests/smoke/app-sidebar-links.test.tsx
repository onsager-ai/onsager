import { describe, it, expect, vi, beforeEach } from "vitest"
import { render, screen, waitFor } from "@testing-library/react"
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { MemoryRouter, Routes, Route } from "react-router-dom"

import { SidebarProvider } from "@/components/ui/sidebar"
import { AppSidebar } from "@/components/layout/AppSidebar"

// AppSidebar is mounted in AppLayout, which sits above the
// `/workspaces/:workspace/*` route — so WorkspaceContext is not in
// scope. The sidebar must still resolve scoped nav links from the URL
// itself; otherwise every sidebar item collapses to the `/workspaces`
// fallback and the active sidebar group never matches the open page.

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
    listAllProjects: vi.fn().mockResolvedValue({ projects: [] }),
    listWorkspaceInstallations: vi
      .fn()
      .mockResolvedValue({ installations: [] }),
  },
}))

function renderAppSidebar(initialPath: string) {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return render(
    <QueryClientProvider client={qc}>
      <MemoryRouter initialEntries={[initialPath]}>
        <Routes>
          <Route
            path="*"
            element={
              <SidebarProvider>
                <AppSidebar />
              </SidebarProvider>
            }
          />
        </Routes>
      </MemoryRouter>
    </QueryClientProvider>,
  )
}

describe("AppSidebar nav link resolution", () => {
  beforeEach(() => vi.clearAllMocks())

  it("scopes the two-surface IA nav links to the workspace in the URL", async () => {
    renderAppSidebar("/workspaces/acme/workflows")

    // Two-surface IA per spec #289: only Chat + Workflows in the nav.
    // Settings is reached via the footer avatar menu; everything else
    // (Sessions, Nodes, Issues, Artifacts, Spine, Governance, Overview)
    // is reachable via direct URL and the ⌘K command palette.
    await waitFor(() => {
      const workflows = screen.getByRole("link", { name: /^Workflows$/ })
      expect(workflows).toHaveAttribute("href", "/workspaces/acme/workflows")
    })

    expect(screen.getByRole("link", { name: /^Chat$/ })).toHaveAttribute(
      "href",
      "/workspaces/acme/chat",
    )
  })

  it("falls back to /workspaces when there is no workspace in the URL", async () => {
    renderAppSidebar("/settings")

    // Outside a scoped route every nav item shares the `/workspaces`
    // picker fallback so the user lands somewhere that can resolve
    // the missing workspace context.
    await waitFor(() => {
      const workflows = screen.getByRole("link", { name: /^Workflows$/ })
      expect(workflows).toHaveAttribute("href", "/workspaces")
    })
    expect(screen.getByRole("link", { name: /^Chat$/ })).toHaveAttribute(
      "href",
      "/workspaces",
    )
  })
})
