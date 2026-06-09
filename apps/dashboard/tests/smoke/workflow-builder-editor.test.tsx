import { describe, it, expect, vi } from "vitest"
import { render, screen, fireEvent } from "@testing-library/react"
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { MemoryRouter } from "react-router-dom"
import type { ReactNode } from "react"
import { StageEditor } from "@/components/workflows/StageEditor"
import { TriggerEditor } from "@/components/workflows/TriggerEditor"
import type { WorkflowStage } from "@/lib/api"
import type { WorkflowTriggerDraft } from "@/components/workflows/workflow-draft"
import {
  draftToRequestTrigger,
  isTriggerReady,
} from "@/components/workflows/workflow-draft"

function mount(node: ReactNode) {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return render(
    <QueryClientProvider client={qc}>
      <MemoryRouter>{node}</MemoryRouter>
    </QueryClientProvider>,
  )
}

// The master-detail builder (#569) renders the node editor inline in the
// right pane — no card→sheet click. These assertions mirror the old
// card-editor smoke checks against the new StageEditor / TriggerEditor.
describe("Stage editor (master-detail right pane)", () => {
  const base: WorkflowStage = {
    id: "s1",
    name: "Agent session",
    gate_kind: "agent-session",
    config: {},
  }

  it("emits a structured gate_kind when a toggle is selected", () => {
    const onChange = vi.fn()
    mount(<StageEditor stage={base} index={0} onChange={onChange} onRemove={() => {}} />)
    fireEvent.click(screen.getByRole("button", { name: /External check/i }))
    expect(onChange).toHaveBeenCalledWith(
      expect.objectContaining({ gate_kind: "external-check" }),
    )
  })

  it("does not expose a free-text input for the gate kind", () => {
    mount(<StageEditor stage={base} index={0} onChange={() => {}} onRemove={() => {}} />)
    // The only text input is the (optional) stage name — a display label.
    // Gate kind is a discrete toggle, not a free-text field.
    const textInputs = screen
      .getAllByRole("textbox")
      .filter((el) => el.getAttribute("id")?.startsWith("stage-name-"))
    expect(textInputs.length).toBe(1)
  })
})

describe("Trigger editor (master-detail right pane)", () => {
  const empty: WorkflowTriggerDraft = {
    kind_tag: "github_issue_webhook",
    install_id: "",
    repo_owner: "",
    repo_name: "",
    label: "",
    manual_name: "",
  }

  const manual: WorkflowTriggerDraft = {
    kind_tag: "manual",
    install_id: "",
    repo_owner: "",
    repo_name: "",
    label: "",
    manual_name: "Run nightly batch",
  }

  it("has no free-text inputs for the linkable fields (install/repo/label)", () => {
    mount(
      <TriggerEditor workspaceId="t1" installations={[]} value={empty} onChange={() => {}} />,
    )
    // A GitHub-webhook trigger exposes only discrete pickers — the repo
    // multi-combobox is a closed popover button (role combobox), not a
    // text field; zero native `<input type="text">` are rendered.
    expect(screen.queryAllByRole("textbox").length).toBe(0)
  })

  it("tells a repo-less (manual) trigger it runs workspace-scoped (#553)", () => {
    mount(
      <TriggerEditor workspaceId="t1" installations={[]} value={manual} onChange={() => {}} />,
    )
    // Mutation guard: delete the Repositories note from the manual branch
    // and this fails.
    expect(
      screen.getByText(/any repository bound to this workspace/i),
    ).toBeInTheDocument()
  })

  it("does not show the workspace-scoped note for a GitHub webhook trigger", () => {
    mount(
      <TriggerEditor workspaceId="t1" installations={[]} value={empty} onChange={() => {}} />,
    )
    // GitHub webhook triggers pin repos via the webhook itself, so the
    // workspace-scoped note must not appear (no regression).
    expect(
      screen.queryByText(/any repository bound to this workspace/i),
    ).not.toBeInTheDocument()
  })
})

describe("workflow-draft serialization", () => {
  it("only produces structured trigger values on the wire", () => {
    const t: WorkflowTriggerDraft = {
      kind_tag: "github_issue_webhook",
      install_id: "inst_1",
      repo_owner: "onsager-ai",
      repo_name: "onsager",
      label: "factory",
      manual_name: "",
    }
    expect(isTriggerReady(t)).toBe(true)
    const wire = draftToRequestTrigger(t)
    expect(wire).toEqual({
      kind: "github-label",
      install_id: "inst_1",
      repo_owner: "onsager-ai",
      repo_name: "onsager",
      label: "factory",
      kind_tag: "github_issue_webhook",
      manual_name: "",
    })
  })

  it("rejects drafts with empty linkable fields", () => {
    expect(
      isTriggerReady({
        kind_tag: "github_issue_webhook",
        install_id: "inst_1",
        repo_owner: "onsager-ai",
        repo_name: "onsager",
        label: "",
        manual_name: "",
      }),
    ).toBe(false)
    expect(
      isTriggerReady({
        kind_tag: "github_issue_webhook",
        install_id: "",
        repo_owner: "a",
        repo_name: "b",
        label: "c",
        manual_name: "",
      }),
    ).toBe(false)
  })
})
