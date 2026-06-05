import { describe, it, expect, vi } from "vitest"
import { render, screen, fireEvent } from "@testing-library/react"
import { PresetPicker } from "@/components/workflows/PresetPicker"
import {
  WORKFLOW_PRESETS,
  emptyDocument,
  type WorkflowDocument,
} from "@/components/workflows/workflow-draft"

// Open the preset dropdown and choose the option whose label matches.
function choosePreset(label: string) {
  fireEvent.click(screen.getByRole("combobox", { name: "Start from a preset" }))
  fireEvent.click(screen.getByText(label))
}

describe("PresetPicker", () => {
  it("renders a single compact preset dropdown, not a button grid", () => {
    render(<PresetPicker draft={emptyDocument()} onApply={() => {}} />)
    // One de-emphasized control: the Select trigger, labeled for a11y.
    expect(
      screen.getByRole("combobox", { name: "Start from a preset" }),
    ).toBeTruthy()
    // The old per-preset buttons are gone.
    for (const p of WORKFLOW_PRESETS) {
      expect(screen.queryByRole("button", { name: new RegExp(p.label) })).toBeNull()
    }
  })

  it("lists every preset (label + description) inside the dropdown", () => {
    render(<PresetPicker draft={emptyDocument()} onApply={() => {}} />)
    fireEvent.click(screen.getByRole("combobox", { name: "Start from a preset" }))
    for (const p of WORKFLOW_PRESETS) {
      expect(screen.getByText(p.label)).toBeTruthy()
      expect(screen.getByText(p.description)).toBeTruthy()
    }
  })

  it("choosing a preset fills stages but preserves the existing trigger", () => {
    const onApply = vi.fn()
    const draft: WorkflowDocument = {
      name: "",
      trigger: {
        install_id: "inst_1",
        repo_owner: "onsager-ai",
        repo_name: "onsager",
        label: "factory",
      },
      stages: [],
    }
    render(<PresetPicker draft={draft} onApply={onApply} />)
    choosePreset("Issue → PR")
    const next = onApply.mock.calls[0][0] as WorkflowDocument
    expect(next.trigger).toEqual(draft.trigger)
    expect(next.stages.length).toBeGreaterThan(0)
  })

  it("preserves a user-typed name instead of overriding with the preset's name", () => {
    const onApply = vi.fn()
    const draft: WorkflowDocument = {
      ...emptyDocument(),
      name: "My custom workflow",
    }
    render(<PresetPicker draft={draft} onApply={onApply} />)
    choosePreset("Issue → PR")
    const next = onApply.mock.calls[0][0] as WorkflowDocument
    expect(next.name).toBe("My custom workflow")
  })

  it("falls back to the preset-generated name when the draft name is blank", () => {
    const onApply = vi.fn()
    render(<PresetPicker draft={emptyDocument()} onApply={onApply} />)
    choosePreset("Issue → PR")
    const next = onApply.mock.calls[0][0] as WorkflowDocument
    expect(next.name.length).toBeGreaterThan(0)
  })

  it("every preset generates a readable name when no repo has been picked yet", () => {
    // Regression: applying the Issue → PR preset before the trigger repo
    // was picked used to produce names like "/ — issue to PR" (empty
    // owner/name around a literal slash). Every preset must fall back to
    // a placeholder instead.
    const { trigger } = emptyDocument()
    for (const p of WORKFLOW_PRESETS) {
      const next = p.build(trigger)
      expect(next.name, `${p.id} name`).not.toMatch(/^\s*\//)
      expect(next.name, `${p.id} name`).not.toMatch(/\/\s+—/)
    }
  })
})
