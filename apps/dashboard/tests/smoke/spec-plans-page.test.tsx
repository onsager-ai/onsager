import { describe, it, expect, vi, beforeEach } from "vitest"
import { render, screen, fireEvent, act, waitFor } from "@testing-library/react"
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { MemoryRouter, Routes, Route } from "react-router-dom"

import { SpecPlansPage } from "@/pages/SpecPlansPage"
import { WorkspaceScope } from "@/lib/workspace"

// Spec Plans surface (ADR 0023 / 0025, spec #514): the page lists
// persisted plans (MCP-only `list_spec_plans`) and each row's "Run"
// button emits `plan.run_requested` via `run_spec_plan` through a
// HitlCard. The mutation check asserts the exact `mcpCallTool` call.

vi.mock("@/lib/auth", () => ({
  useAuth: () => ({ user: { id: "user-1" }, authEnabled: false }),
}))

const apiMock = vi.hoisted(() => ({ listWorkspaces: vi.fn() }))
vi.mock("@/lib/api", async () => {
  const actual =
    await vi.importActual<typeof import("@/lib/api")>("@/lib/api")
  return {
    ...actual,
    api: { ...actual.api, listWorkspaces: apiMock.listWorkspaces },
  }
})

const mcpMock = vi.hoisted(() => ({ mcpCallTool: vi.fn() }))
vi.mock("@/lib/mcp-client", async () => {
  const actual =
    await vi.importActual<typeof import("@/lib/mcp-client")>("@/lib/mcp-client")
  return { ...actual, mcpCallTool: mcpMock.mcpCallTool }
})

const PLANS = {
  spec_plans: [
    {
      workspace_id: "ws_1",
      spec_plan_id: "plan-alpha",
      plan: {
        specs: [{ id: "s1", kind: "Issue" }],
        deps: [],
      },
      created_by: "user-1",
      created_at: "2026-05-29T00:00:00Z",
      updated_at: "2026-05-29T00:00:00Z",
    },
  ],
}

beforeEach(() => {
  vi.clearAllMocks()
  apiMock.listWorkspaces.mockResolvedValue({
    workspaces: [
      {
        id: "ws_1",
        slug: "acme",
        name: "Acme",
        account: "acme",
        account_type: "organization" as const,
      },
    ],
  })
  mcpMock.mcpCallTool.mockImplementation((name: string) => {
    if (name === "list_spec_plans") {
      return Promise.resolve({
        content: [{ type: "text", text: JSON.stringify(PLANS) }],
        isError: false,
      })
    }
    if (name === "run_spec_plan") {
      return Promise.resolve({
        content: [
          {
            type: "text",
            text: JSON.stringify({
              spec_plan_id: "plan-alpha",
              workspace_id: "ws_1",
              plan_id: "run-xyz",
              run_requested_event_id: 9,
            }),
          },
        ],
        isError: false,
      })
    }
    return Promise.resolve({ content: [], isError: false })
  })
})

function mount() {
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  return render(
    <QueryClientProvider client={qc}>
      <MemoryRouter initialEntries={["/workspaces/acme/spec-plans"]}>
        <Routes>
          <Route
            path="/workspaces/:workspace/*"
            element={
              <WorkspaceScope>
                <Routes>
                  <Route path="spec-plans" element={<SpecPlansPage />} />
                </Routes>
              </WorkspaceScope>
            }
          />
        </Routes>
      </MemoryRouter>
    </QueryClientProvider>,
  )
}

describe("SpecPlansPage (spec #514)", () => {
  it("lists persisted plans and runs one via run_spec_plan", async () => {
    mount()

    // The persisted plan renders.
    await screen.findByText("plan-alpha")
    expect(mcpMock.mcpCallTool).toHaveBeenCalledWith("list_spec_plans", {
      workspace_id: "ws_1",
    })

    // Run → HitlCard review dialog → commit.
    const runBtn = screen.getByRole("button", { name: "Run" })
    await act(async () => {
      fireEvent.click(runBtn)
    })
    const commit = await screen.findByRole("button", { name: /run plan/i })
    await act(async () => {
      fireEvent.click(commit)
    })

    await waitFor(() => {
      expect(mcpMock.mcpCallTool).toHaveBeenCalledWith("run_spec_plan", {
        workspace_id: "ws_1",
        spec_plan_id: "plan-alpha",
      })
    })
  })

  it("shows an empty state when there are no plans", async () => {
    mcpMock.mcpCallTool.mockResolvedValueOnce({
      content: [{ type: "text", text: JSON.stringify({ spec_plans: [] }) }],
      isError: false,
    })
    mount()
    await screen.findByText(/no spec plans yet/i)
  })
})
