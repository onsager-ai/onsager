import { describe, it, expect, vi } from "vitest"
import { render, screen, fireEvent } from "@testing-library/react"
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { MemoryRouter } from "react-router-dom"
import type { ReactNode } from "react"
import { StageCard } from "@/components/workflows/StageCard"
import { TriggerCard } from "@/components/workflows/TriggerCard"
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

describe("Stage card editor", () => {
  const base: WorkflowStage = {
    id: "s1",
    name: "Agent session",
    gate_kind: "agent-session",
    artifact_kind: "github-issue",
    config: {},
  }

  it("emits a structured gate_kind when a toggle is selected", () => {
    const onChange = vi.fn()
    mount(<StageCard stage={base} index={0} onChange={onChange} onRemove={() => {}} />)
    fireEvent.click(screen.getByRole("button", { name: /Edit stage/i }))
    fireEvent.click(screen.getByRole("button", { name: /External check/i }))
    expect(onChange).toHaveBeenCalledWith(
      expect.objectContaining({ gate_kind: "external-check" }),
    )
  })

  it("does not expose a free-text input for the gate kind", () => {
    mount(
      <StageCard stage={base} index={0} onChange={() => {}} onRemove={() => {}} />,
    )
    fireEvent.click(screen.getByRole("button", { name: /Edit stage/i }))
    // There are no <input type="text"> fields that accept arbitrary gate
    // strings; the only text input is the stage name (a display label).
    const textInputs = screen
      .getAllByRole("textbox")
      .filter((el) => el.getAttribute("id")?.startsWith("stage-name-"))
    expect(textInputs.length).toBe(1)
  })
})

describe("Trigger card editor", () => {
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
      <TriggerCard
        workspaceId="t1"
        installations={[]}
        value={empty}
        onChange={() => {}}
      />,
    )
    fireEvent.click(screen.getByRole("button", { name: /Edit trigger/i }))
    // The trigger sheet exposes only discrete pickers — Sheet content
    // should contain zero native `<input type="text">` fields.
    const textboxes = screen.queryAllByRole("textbox")
    expect(textboxes.length).toBe(0)
  })

  it("tells a repo-less (manual) trigger it runs workspace-scoped (#553)", () => {
    mount(
      <TriggerCard
        workspaceId="t1"
        installations={[]}
        value={manual}
        onChange={() => {}}
      />,
    )
    fireEvent.click(screen.getByRole("button", { name: /Edit trigger/i }))
    // A repo-less workflow defaults to "any bound repo" — the form states
    // that runtime behavior rather than pinning a repo. Mutation guard:
    // delete the Repositories note from the manual branch and this fails.
    expect(
      screen.getByText(/any repository bound to this workspace/i),
    ).toBeInTheDocument()
  })

  it("does not show the workspace-scoped note for a GitHub webhook trigger", () => {
    mount(
      <TriggerCard
        workspaceId="t1"
        installations={[]}
        value={empty}
        onChange={() => {}}
      />,
    )
    fireEvent.click(screen.getByRole("button", { name: /Edit trigger/i }))
    // GitHub webhook triggers pin exactly one repo via the webhook itself,
    // so the workspace-scoped note must not appear (no regression).
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
