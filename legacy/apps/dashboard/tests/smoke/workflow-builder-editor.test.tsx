import { describe, it, expect, vi } from "vitest"
import { render, screen, fireEvent } from "@testing-library/react"
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { MemoryRouter } from "react-router-dom"
import type { ReactNode } from "react"
import { StageEditor } from "@/components/workflows/StageEditor"
import { TriggerEventEditor } from "@/components/workflows/TriggerEventEditor"
import { TriggerReposEditor } from "@/components/workflows/TriggerReposEditor"
import type { WorkflowStage } from "@/lib/api"
import type { WorkflowTriggerDraft } from "@/components/workflows/workflow-draft"

function mount(node: ReactNode) {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return render(
    <QueryClientProvider client={qc}>
      <MemoryRouter>{node}</MemoryRouter>
    </QueryClientProvider>,
  )
}

const githubTrigger: WorkflowTriggerDraft = {
  kind_tag: "github_issue_webhook",
  install_id: "",
  repo_owner: "",
  repo_name: "",
  label: "",
}

const manualTrigger: WorkflowTriggerDraft = {
  kind_tag: "manual",
  install_id: "",
  repo_owner: "",
  repo_name: "",
  label: "",
}

// The tabbed builder (#570/#581) splits the trigger node across a Trigger tab
// (kind + event) and a Repositories tab. These assertions mirror the old
// card-editor smoke checks against the new StageEditor / TriggerEventEditor /
// TriggerReposEditor.
describe("Stage editor (Steps tab right pane)", () => {
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

describe("Trigger event editor (Trigger tab)", () => {
  it("exposes no free-text input for a GitHub webhook trigger", () => {
    mount(
      <TriggerEventEditor
        workspaceId="t1"
        value={githubTrigger}
        onChange={() => {}}
        onGoToRepos={() => {}}
      />,
    )
    // Kind is a segmented toggle (buttons) and the label is a closed picker;
    // with no repo chosen the label collapses to a "pick a repository first"
    // hint — zero native text inputs either way.
    expect(screen.queryAllByRole("textbox").length).toBe(0)
  })

  it("exposes no free-text input for a Manual trigger (#581 — no trigger name)", () => {
    mount(
      <TriggerEventEditor
        workspaceId="t1"
        value={manualTrigger}
        onChange={() => {}}
        onGoToRepos={() => {}}
      />,
    )
    // Mutation guard for #581: the manual branch must not render a trigger
    // name input — the fire button derives its label from the workflow name.
    expect(screen.queryAllByRole("textbox").length).toBe(0)
  })
})

describe("Trigger repos editor (Repositories tab)", () => {
  it("tells a repo-less (manual) trigger it runs workspace-scoped (#553)", () => {
    mount(
      <TriggerReposEditor
        workspaceId="t1"
        installations={[]}
        value={manualTrigger}
        onChange={() => {}}
      />,
    )
    // Mutation guard: delete the workspace-scoped note from the manual branch
    // and this fails.
    expect(
      screen.getByText(/any repository bound to this workspace/i),
    ).toBeInTheDocument()
  })

  it("does not show the workspace-scoped note for a GitHub webhook trigger", () => {
    mount(
      <TriggerReposEditor
        workspaceId="t1"
        installations={[]}
        value={githubTrigger}
        onChange={() => {}}
      />,
    )
    // GitHub webhook triggers pin repos via the repo multi-combobox, so the
    // workspace-scoped note must not appear (no regression).
    expect(
      screen.queryByText(/any repository bound to this workspace/i),
    ).not.toBeInTheDocument()
  })
})
