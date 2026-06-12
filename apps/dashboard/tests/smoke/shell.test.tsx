import { describe, expect, it, vi, beforeEach } from "vitest"
import { render, screen, waitFor } from "@testing-library/react"
import { MemoryRouter } from "react-router-dom"
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"

vi.mock("@/lib/api", async () => {
  const actual = await vi.importActual<typeof import("@/lib/api")>("@/lib/api")
  return {
    ...actual,
    workflowsApi: {
      list: vi.fn().mockResolvedValue({ workflows: [] }),
    },
    api: {
      authProviders: vi.fn().mockResolvedValue({ github: false, dev: true }),
      getMe: vi.fn().mockResolvedValue({
        session_kind: "dev",
        user: { id: "u1", github_login: "marvin@local", github_name: null, github_avatar_url: null },
      }),
      devLogin: vi.fn(),
      logout: vi.fn(),
      listWorkspaces: vi.fn().mockResolvedValue({
        workspaces: [
          { id: "w1", slug: "dev", name: "Dev workspace", created_by: "u1", created_at: "2026-06-12T00:00:00Z" },
        ],
      }),
    },
  }
})

import { LoginPage } from "@/pages/LoginPage"
import { WorkflowsPage } from "@/pages/WorkflowsPage"
import { WorkspaceProvider } from "@/lib/workspace"
import { AuthProvider } from "@/lib/auth"

function wrap(node: React.ReactNode) {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return render(
    <QueryClientProvider client={qc}>
      <AuthProvider>
        <MemoryRouter>{node}</MemoryRouter>
      </AuthProvider>
    </QueryClientProvider>,
  )
}

beforeEach(() => vi.clearAllMocks())

describe("v2 shell (M0 #622)", () => {
  it("login page offers dev-login when the provider says so", async () => {
    wrap(<LoginPage />)
    await waitFor(() => expect(screen.getByText(/dev login/i)).toBeInTheDocument())
  })

  it("workflows page renders against the resolved workspace", async () => {
    wrap(
      <WorkspaceProvider>
        <WorkflowsPage />
      </WorkspaceProvider>,
    )
    await waitFor(() =>
      expect(screen.getByText(/no workflows yet/i)).toBeInTheDocument(),
    )
  })
})
