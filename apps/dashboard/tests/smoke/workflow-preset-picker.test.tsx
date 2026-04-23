import { describe, it, expect, vi } from "vitest"
import { render, screen, fireEvent } from "@testing-library/react"
import { PresetPicker } from "@/components/factory/workflows/PresetPicker"
import {
  WORKFLOW_PRESETS,
  emptyDraft,
  type WorkflowDraft,
} from "@/components/factory/workflows/workflow-draft"

describe("PresetPicker", () => {
  it("renders one button per preset", () => {
    render(<PresetPicker draft={emptyDraft()} onApply={() => {}} />)
    for (const p of WORKFLOW_PRESETS) {
      expect(screen.getByRole("button", { name: new RegExp(p.label) })).toBeTruthy()
    }
  })

  it("applying a preset fills stages but preserves the existing trigger", () => {
    const onApply = vi.fn()
    const draft: WorkflowDraft = {
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
    fireEvent.click(screen.getByRole("button", { name: /Issue → PR/ }))
    const next = onApply.mock.calls[0][0] as WorkflowDraft
    expect(next.trigger).toEqual(draft.trigger)
    expect(next.stages.length).toBeGreaterThan(0)
  })

  it("preserves a user-typed name instead of overriding with the preset's name", () => {
    const onApply = vi.fn()
    const draft: WorkflowDraft = {
      ...emptyDraft(),
      name: "My custom workflow",
    }
    render(<PresetPicker draft={draft} onApply={onApply} />)
    fireEvent.click(screen.getByRole("button", { name: /Issue → PR/ }))
    const next = onApply.mock.calls[0][0] as WorkflowDraft
    expect(next.name).toBe("My custom workflow")
  })

  it("falls back to the preset-generated name when the draft name is blank", () => {
    const onApply = vi.fn()
    render(<PresetPicker draft={emptyDraft()} onApply={onApply} />)
    fireEvent.click(screen.getByRole("button", { name: /Issue → PR/ }))
    const next = onApply.mock.calls[0][0] as WorkflowDraft
    expect(next.name.length).toBeGreaterThan(0)
  })

  it("every preset generates a readable name when no repo has been picked yet", () => {
    // Regression: applying the Issue → PR preset before the trigger repo
    // was picked used to produce names like "/ — issue to PR" (empty
    // owner/name around a literal slash). Every preset must fall back to
    // a placeholder instead.
    const { trigger } = emptyDraft()
    for (const p of WORKFLOW_PRESETS) {
      const next = p.build(trigger)
      expect(next.name, `${p.id} name`).not.toMatch(/^\s*\//)
      expect(next.name, `${p.id} name`).not.toMatch(/\/\s+—/)
    }
  })
})
