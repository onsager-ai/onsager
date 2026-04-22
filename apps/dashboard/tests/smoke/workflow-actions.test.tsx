import { describe, it, expect, vi, beforeEach, afterEach } from "vitest"
import { render, screen, fireEvent, waitFor } from "@testing-library/react"
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { MemoryRouter } from "react-router-dom"
import type { ReactNode } from "react"
import { WorkflowActions } from "@/components/factory/workflows/WorkflowActions"
import { api, type Workflow } from "@/lib/api"

function mount(node: ReactNode) {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return render(
    <QueryClientProvider client={qc}>
      <MemoryRouter>{node}</MemoryRouter>
    </QueryClientProvider>,
  )
}

function makeWorkflow(overrides: Partial<Workflow> = {}): Workflow {
  return {
    id: "wf_1",
    tenant_id: "t_1",
    name: "Issue → PR",
    preset: null,
    status: "active",
    trigger: {
      kind: "github-label",
      install_id: "42",
      repo_owner: "onsager-ai",
      repo_name: "onsager",
      label: "factory",
    },
    stages: [],
    created_at: "2026-04-22T00:00:00Z",
    updated_at: "2026-04-22T00:00:00Z",
    ...overrides,
  }
}

describe("WorkflowActions lifecycle controls", () => {
  beforeEach(() => {
    vi.spyOn(api, "setWorkflowActive").mockResolvedValue({
      workflow: makeWorkflow(),
    })
    vi.spyOn(api, "deleteWorkflow").mockResolvedValue({ ok: true })
  })
  afterEach(() => {
    vi.restoreAllMocks()
  })

  it("pauses an active workflow via PATCH active:false", async () => {
    mount(<WorkflowActions workflow={makeWorkflow({ status: "active" })} />)
    fireEvent.click(screen.getByRole("button", { name: /Pause/i }))
    await waitFor(() =>
      expect(api.setWorkflowActive).toHaveBeenCalledWith("wf_1", false),
    )
  })

  it("labels the toggle as 'Publish' on a draft workflow", () => {
    mount(<WorkflowActions workflow={makeWorkflow({ status: "draft" })} />)
    expect(screen.getByRole("button", { name: /Publish/i })).toBeTruthy()
  })

  it("labels the toggle as 'Resume' on a paused workflow", () => {
    mount(<WorkflowActions workflow={makeWorkflow({ status: "paused" })} />)
    expect(screen.getByRole("button", { name: /Resume/i })).toBeTruthy()
  })

  it("requires confirmation before DELETE fires", async () => {
    mount(<WorkflowActions workflow={makeWorkflow()} />)
    fireEvent.click(screen.getByRole("button", { name: /^Delete$/i }))
    // The modal's destructive button is the one that actually calls the API.
    const confirm = await screen.findByRole("button", {
      name: /Delete workflow/i,
    })
    fireEvent.click(confirm)
    await waitFor(() => expect(api.deleteWorkflow).toHaveBeenCalledWith("wf_1"))
  })
})
