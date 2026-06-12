import { describe, it, expect, vi, beforeEach } from "vitest"
import {
  render,
  screen,
  fireEvent,
  act,
  waitFor,
  within,
} from "@testing-library/react"
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { MemoryRouter, Routes, Route, useLocation } from "react-router-dom"

import { CreateSpecPlanPage } from "@/pages/CreateSpecPlanPage"
import { WorkspaceScope } from "@/lib/workspace"

// Guided "Create Plan" surface (spec #542). Asserts the authoring loop:
// form rows → live `compile_dry_run` gate → `submit_spec_plan` via a
// HitlCard. The compile gate is mutation-checked — a kind the library
// rejects keeps the commit button disabled, and the exact submit call is
// asserted so dropping the gate fails the suite.

vi.mock("@/lib/auth", () => ({
  useAuth: () => ({ user: { id: "u1" }, authEnabled: false }),
}))

vi.mock("@/lib/api", () => ({
  api: {
    listWorkspaces: vi.fn().mockResolvedValue({
      workspaces: [
        {
          id: "w1",
          slug: "acme",
          name: "Acme",
          account: "acme",
          account_type: "organization" as const,
        },
      ],
    }),
  },
}))

const mcpMock = vi.hoisted(() => ({ mcpCallTool: vi.fn() }))
vi.mock("@/lib/mcp-client", async () => {
  const actual =
    await vi.importActual<typeof import("@/lib/mcp-client")>("@/lib/mcp-client")
  return { ...actual, mcpCallTool: mcpMock.mcpCallTool }
})

function ok(payload: unknown) {
  return Promise.resolve({
    content: [{ type: "text", text: JSON.stringify(payload) }],
    isError: false,
  })
}

beforeEach(() => {
  vi.clearAllMocks()
  mcpMock.mcpCallTool.mockImplementation(
    (name: string, args: Record<string, unknown>) => {
      if (name === "list_workflows_v2") {
        return ok({
          workflows: [
            { spec_kind: "Issue", version: 1, retired_at: null },
            { spec_kind: "PR", version: 1, retired_at: null },
            { spec_kind: "broken", version: 1, retired_at: null },
          ],
        })
      }
      if (name === "compile_dry_run") {
        const plan = args.spec_plan as { specs: { kind: string }[] }
        const bad = plan.specs.some((s) => s.kind === "broken")
        return bad
          ? ok({
              ok: false,
              error: {
                kind: "missing_kind",
                message: "unknown spec kind `broken`",
                spec_kind: "broken",
              },
            })
          : ok({ ok: true, execution_plan: { node_count: 1, edge_count: 0 } })
      }
      if (name === "submit_spec_plan") {
        return ok({ spec_plan: { spec_plan_id: args.spec_plan_id } })
      }
      return Promise.resolve({ content: [], isError: false })
    },
  )
})

function LocationProbe() {
  const loc = useLocation()
  return <div data-testid="location">{loc.pathname}</div>
}

function mount() {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return render(
    <QueryClientProvider client={qc}>
      <MemoryRouter initialEntries={["/workspaces/acme/spec-plans/new"]}>
        <Routes>
          <Route
            path="/workspaces/:workspace/*"
            element={
              <WorkspaceScope>
                <Routes>
                  <Route path="spec-plans/new" element={<CreateSpecPlanPage />} />
                  <Route path="spec-plans" element={<div>plans list</div>} />
                </Routes>
                <LocationProbe />
              </WorkspaceScope>
            }
          />
        </Routes>
      </MemoryRouter>
    </QueryClientProvider>,
  )
}

async function fillPlan(specId: string, kind: string) {
  fireEvent.change(await screen.findByLabelText("Plan ID"), {
    target: { value: "p1" },
  })
  fireEvent.change(screen.getByLabelText("Spec 1 id"), {
    target: { value: specId },
  })
  // Kind is a library-backed combobox, never a typed string.
  fireEvent.click(screen.getByRole("combobox", { name: "Spec 1 kind" }))
  fireEvent.click(await screen.findByText(kind))
}

function trigger() {
  return document.querySelector(
    '[data-slot="hitl-action-button"][data-tool="submit_spec_plan"]',
  ) as HTMLButtonElement
}

describe("CreateSpecPlanPage (spec #542)", () => {
  it("compiles a valid plan and commits via submit_spec_plan", async () => {
    mount()
    await act(async () => {
      await fillPlan("github:1", "Issue")
    })

    // Live compile lands ok → the commit button enables.
    await waitFor(() => expect(trigger()).not.toBeDisabled())

    await act(async () => {
      fireEvent.click(trigger())
    })
    const dialog = await screen.findByRole("dialog")
    await act(async () => {
      fireEvent.click(
        within(dialog).getByRole("button", { name: /submit plan/i }),
      )
    })

    await waitFor(() =>
      expect(mcpMock.mcpCallTool).toHaveBeenCalledWith("submit_spec_plan", {
        workspace_id: "w1",
        spec_plan_id: "p1",
        spec_plan: { specs: [{ id: "github:1", kind: "Issue" }], deps: [] },
      }),
    )
    // On success the surface routes back to the plans list.
    await waitFor(() =>
      expect(screen.getByTestId("location")).toHaveTextContent(
        "/workspaces/acme/spec-plans",
      ),
    )
  })

  it("disables commit when the compiler rejects the plan", async () => {
    mount()
    await act(async () => {
      await fillPlan("github:1", "broken")
    })

    // The compile error surfaces and the gate stays closed.
    await screen.findByText(/unknown spec kind `broken`/i)
    expect(trigger()).toBeDisabled()
  })
})
