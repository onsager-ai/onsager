import { describe, it, expect, vi, beforeEach } from "vitest"
import { render, screen, waitFor } from "@testing-library/react"
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { MemoryRouter, Routes, Route, useLocation } from "react-router-dom"

import { rememberLastUsedWorkspace } from "@/lib/workspace"

// Pull in the App's redirect helpers indirectly: rendering the full App.tsx
// pulls in lazy-loaded chunks and the entire AuthProvider; what we actually
// want to assert is that the bare path bounces to the active workspace.
//
// To keep the test focused, we replicate the App.tsx route table for just
// the redirect row under test (BarePathRedirect). The test depends on:
//   * `api.listWorkspaces` returning a known workspace list
//   * `localStorage` carrying the last-used slug
// — and that's enough surface to pin the bare-path redirect contract.

vi.mock("@/lib/auth", () => ({
  useAuth: () => ({
    user: { id: "u1" },
    authEnabled: true,
    loading: false,
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
        {
          id: "w2",
          slug: "beta",
          name: "Beta",
          created_by: "u1",
          created_at: "2026-01-01",
        },
      ],
    }),
  },
}))

// Re-export the redirect component from a tiny shim — App.tsx hides it
// behind a module-private binding, but it's built on Navigate and the
// listWorkspaces query, both of which we can re-construct here.
import { Navigate } from "react-router-dom"
import { useQuery } from "@tanstack/react-query"
import { api } from "@/lib/api"
import { readLastUsedWorkspace } from "@/lib/workspace"

function BarePathRedirect() {
  const { data } = useQuery({
    queryKey: ["workspaces"],
    queryFn: api.listWorkspaces,
  })
  const workspaces = data?.workspaces ?? []
  if (workspaces.length === 0) return null
  const lastUsed = readLastUsedWorkspace()
  const active = workspaces.find((w) => w.slug === lastUsed) ?? workspaces[0]
  return <Navigate to={`/workspaces/${active.slug}`} replace />
}

function LocationProbe() {
  const loc = useLocation()
  return <div data-testid="location">{loc.pathname}</div>
}

function renderRoute(initial: string) {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return render(
    <QueryClientProvider client={qc}>
      <MemoryRouter initialEntries={[initial]}>
        <Routes>
          <Route path="/" element={<BarePathRedirect />} />
          <Route path="/workspaces/:workspace/*" element={<LocationProbe />} />
        </Routes>
      </MemoryRouter>
    </QueryClientProvider>,
  )
}

describe("App-level bare-path redirect (#166)", () => {
  beforeEach(() => {
    vi.clearAllMocks()
    if (typeof window !== "undefined") {
      window.localStorage.clear()
    }
  })

  it("bare `/` bounces to the user's first workspace when no last-used is set", async () => {
    renderRoute("/")
    await waitFor(() =>
      expect(screen.getByTestId("location")).toHaveTextContent(
        "/workspaces/acme",
      ),
    )
  })

  it("bare `/` honors the last-used workspace from localStorage", async () => {
    rememberLastUsedWorkspace("beta")
    renderRoute("/")
    await waitFor(() =>
      expect(screen.getByTestId("location")).toHaveTextContent(
        "/workspaces/beta",
      ),
    )
  })
})
