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

  it("scopes nav links to the workspace in the URL", async () => {
    renderAppSidebar("/workspaces/acme/sessions")

    await waitFor(() => {
      const sessions = screen.getByRole("link", { name: /Sessions/i })
      expect(sessions).toHaveAttribute("href", "/workspaces/acme/sessions")
    })

    expect(
      screen.getByRole("link", { name: /^Workflows$/ }),
    ).toHaveAttribute("href", "/workspaces/acme/workflows")
    expect(screen.getByRole("link", { name: /Settings/i })).toHaveAttribute(
      "href",
      "/workspaces/acme/settings",
    )
    // Overview row uses an empty suffix; it should land on the workspace
    // root rather than the global picker.
    expect(screen.getByRole("link", { name: /Overview/i })).toHaveAttribute(
      "href",
      "/workspaces/acme",
    )
  })

  it("falls back to /workspaces when there is no workspace in the URL", async () => {
    renderAppSidebar("/settings")

    // Wait for the workspaces query to resolve so the system row renders.
    await waitFor(() => {
      const settings = screen.getByRole("link", { name: /Settings/i })
      expect(settings).toHaveAttribute("href", "/workspaces")
    })
  })
})
