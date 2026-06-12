import { describe, it, expect, vi, beforeEach } from "vitest"
import { render, screen, fireEvent, act, waitFor } from "@testing-library/react"
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { MemoryRouter } from "react-router-dom"

import { WorkflowActions } from "@/components/workflows/WorkflowActions"
import type { Workflow } from "@/lib/api"

// ADR 0024 / 0025, spec #514: the "Run a batch" button gates on the
// `readiness` axis (not `active`) and routes through a HitlCard +
// `run_workflow` MCP tool. The mutation check below asserts the exact
// `mcpCallTool` invocation — breaking the args wiring fails the test.

const mcpMock = vi.hoisted(() => ({ mcpCallTool: vi.fn() }))

vi.mock("@/lib/mcp-client", async () => {
  const actual =
    await vi.importActual<typeof import("@/lib/mcp-client")>("@/lib/mcp-client")
  return { ...actual, mcpCallTool: mcpMock.mcpCallTool }
})

function manualWorkflow(overrides: Partial<Workflow> = {}): Workflow {
  return {
    id: "wf_1",
    workspace_id: "ws_1",
    name: "Nightly batch",
    preset: null,
    status: "active",
    readiness: "ready",
    autofire: "off",
    trigger: {
      kind: "github-label",
      install_id: "",
      repo_owner: "onsager-ai",
      repo_name: "onsager",
      label: "",
      kind_tag: "manual",
      manual_name: "nightly",
    },
    stages: [],
    created_at: "2026-05-01T00:00:00Z",
    updated_at: "2026-05-01T00:00:00Z",
    ...overrides,
  }
}

function renderActions(workflow: Workflow) {
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  return render(
    <QueryClientProvider client={qc}>
      <MemoryRouter>
        <WorkflowActions workflow={workflow} variant="buttons" />
      </MemoryRouter>
    </QueryClientProvider>,
  )
}

describe("WorkflowActions — Run a batch (spec #514)", () => {
  beforeEach(() => {
    vi.clearAllMocks()
    mcpMock.mcpCallTool.mockResolvedValue({
      content: [
        {
          type: "text",
          text: JSON.stringify({
            workflow_id: "wf_1",
            trigger_event_id: 7,
            fired_at: "2026-05-30T00:00:00Z",
          }),
        },
      ],
      isError: false,
    })
  })

  it("shows the button for a ready manual workflow and fires run_workflow", async () => {
    renderActions(manualWorkflow())

    const runBtn = screen.getByRole("button", { name: /run a batch/i })
    await act(async () => {
      fireEvent.click(runBtn)
    })

    // The HitlCard review dialog renders the destructive commit button.
    const commit = await screen.findByRole("button", { name: /run workflow/i })
    await act(async () => {
      fireEvent.click(commit)
    })

    await waitFor(() => {
      expect(mcpMock.mcpCallTool).toHaveBeenCalledWith("run_workflow", {
        workflow_id: "wf_1",
        trigger_name: "nightly",
      })
    })
  })

  it("hides the button for a drafted workflow (gate on readiness)", () => {
    renderActions(manualWorkflow({ readiness: "drafted" }))
    expect(
      screen.queryByRole("button", { name: /run a batch/i }),
    ).toBeNull()
  })

  it("hides the button for a non-manual ready workflow", () => {
    renderActions(
      manualWorkflow({
        trigger: {
          kind: "github-label",
          install_id: "42",
          repo_owner: "onsager-ai",
          repo_name: "onsager",
          label: "factory",
          kind_tag: "github_issue_webhook",
          manual_name: "",
        },
      }),
    )
    expect(
      screen.queryByRole("button", { name: /run a batch/i }),
    ).toBeNull()
  })
})
