import { describe, it, expect, vi, beforeEach } from "vitest"
import { render, screen, fireEvent, waitFor, within } from "@testing-library/react"
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { MemoryRouter } from "react-router-dom"
import type { ReactNode } from "react"

import { WorkflowBuilder } from "@/components/workflows/WorkflowBuilder"
import {
  makeStage,
  type WorkflowDocument,
} from "@/components/workflows/workflow-draft"
import type {
  CreateWorkflowRequest,
  GitHubAppInstallation,
} from "@/lib/api"

const installations: GitHubAppInstallation[] = [
  {
    id: "inst_one",
    workspace_id: "ws_1",
    install_id: 1001,
    account_login: "onsager-ai",
    account_type: "organization",
    created_at: "2026-01-01T00:00:00Z",
  },
]

// A fully-ready GitHub-webhook draft so `canSave` is true and the Create
// button is enabled.
function readyDraft(): WorkflowDocument {
  return {
    name: "Issue → PR",
    trigger: {
      kind_tag: "github_issue_webhook",
      install_id: "inst_one",
      repo_owner: "onsager-ai",
      repo_name: "onsager",
      label: "factory",
      manual_name: "",
    },
    stages: [makeStage("agent-session", "Spec → PR")],
  }
}

// A Manual ("no automatic trigger") draft with a blank button label — the
// default shape (#572): ready with just a name + one stage.
function manualReadyDraft(): WorkflowDocument {
  return {
    name: "Nightly batch",
    trigger: {
      kind_tag: "manual",
      install_id: "",
      repo_owner: "",
      repo_name: "",
      label: "",
      manual_name: "",
    },
    stages: [makeStage("agent-session", "Run")],
  }
}

vi.mock("@/lib/api", async () => {
  const actual = await vi.importActual<object>("@/lib/api")
  return {
    ...actual,
    api: {
      createWorkflow: vi.fn(),
    },
  }
})

async function primeApi() {
  const { api } = await import("@/lib/api")
  vi.mocked(api.createWorkflow).mockResolvedValue({
    workflow: {
      id: "wf_1",
      workspace_id: "ws_1",
      name: "Issue → PR",
      preset: null,
      status: "draft",
      trigger: {
        kind: "github-label",
        install_id: "1001",
        repo_owner: "onsager-ai",
        repo_name: "onsager",
        label: "factory",
        kind_tag: "github_issue_webhook",
        manual_name: "",
      },
      stages: [],
      created_at: "2026-01-01T00:00:00Z",
      updated_at: "2026-01-01T00:00:00Z",
    },
  })
}

function mount(node: ReactNode) {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return render(
    <QueryClientProvider client={qc}>
      <MemoryRouter>{node}</MemoryRouter>
    </QueryClientProvider>,
  )
}

async function lastCreateBody(): Promise<CreateWorkflowRequest> {
  const { api } = await import("@/lib/api")
  const mock = vi.mocked(api.createWorkflow)
  await waitFor(() => expect(mock).toHaveBeenCalledTimes(1))
  return mock.mock.calls[0][0]
}

describe("WorkflowBuilder create action", () => {
  beforeEach(async () => {
    vi.clearAllMocks()
    await primeApi()
  })

  it("renders one Create workflow button and an Active toggle (no draft/activate split)", () => {
    mount(
      <WorkflowBuilder
        workspaceId="ws_1"
        installations={installations}
        initialDraft={readyDraft()}
      />,
    )
    expect(screen.getByRole("button", { name: "Create workflow" })).toBeTruthy()
    expect(screen.getByRole("switch", { name: "Active" })).toBeTruthy()
    expect(screen.queryByRole("button", { name: /Save as draft/i })).toBeNull()
    expect(screen.queryByRole("button", { name: /Activate workflow/i })).toBeNull()
  })

  it("defaults the Active toggle off — Create yields an inactive workflow", async () => {
    mount(
      <WorkflowBuilder
        workspaceId="ws_1"
        installations={installations}
        initialDraft={readyDraft()}
      />,
    )
    fireEvent.click(screen.getByRole("button", { name: "Create workflow" }))
    const body = await lastCreateBody()
    expect(body.active).toBe(false)
  })

  it("threads the Active toggle through to the create request's active boolean", async () => {
    mount(
      <WorkflowBuilder
        workspaceId="ws_1"
        installations={installations}
        initialDraft={readyDraft()}
      />,
    )
    // Flip the toggle on, then create.
    fireEvent.click(screen.getByRole("switch", { name: "Active" }))
    fireEvent.click(screen.getByRole("button", { name: "Create workflow" }))
    const body = await lastCreateBody()
    expect(body.active).toBe(true)
  })

  // The trigger is its own always-visible section, not a node in the stage
  // rail (#572). Mutation guard: re-add a trigger row to FlowRail and the
  // "not inside the rail" assertion fails.
  it("renders the trigger as a section, not a node in the stage rail", () => {
    mount(
      <WorkflowBuilder
        workspaceId="ws_1"
        installations={installations}
        initialDraft={manualReadyDraft()}
      />,
    )
    // The trigger's framing copy is present at the top level…
    const triggerNote = screen.getByText(/no automatic trigger/i)
    expect(triggerNote).toBeTruthy()
    // …but never inside the stage rail.
    const rail = screen.getByRole("navigation", { name: /workflow steps/i })
    expect(within(rail).queryByText(/no automatic trigger/i)).toBeNull()
  })

  // The "no automatic trigger" default (#572): a Manual workflow with a blank
  // button label is saveable, and the wire `name` falls back to the workflow
  // name so the run button is still labeled.
  it("creates a Manual workflow whose button label falls back to the workflow name", async () => {
    mount(
      <WorkflowBuilder
        workspaceId="ws_1"
        installations={installations}
        initialDraft={manualReadyDraft()}
      />,
    )
    fireEvent.click(screen.getByRole("button", { name: "Create workflow" }))
    const body = await lastCreateBody()
    expect(body.trigger).toEqual({ kind: "manual", name: "Nightly batch" })
    expect(body.active).toBe(false)
  })
})
