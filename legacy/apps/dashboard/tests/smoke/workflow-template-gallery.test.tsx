import { describe, it, expect, vi } from "vitest"
import { render, screen, fireEvent } from "@testing-library/react"
import { TemplateGallery } from "@/components/workflows/TemplateGallery"
import {
  WORKFLOW_PRESETS,
  emptyDocument,
  type WorkflowDocument,
} from "@/components/workflows/workflow-draft"

// Template-first create flow (#570/#581): presets moved out of an in-form
// dropdown into the gallery that opens the dialog. Picking a card seeds the
// draft and drops the user into the tabbed editor.

describe("TemplateGallery", () => {
  it("renders a Blank card plus a card per preset", () => {
    render(<TemplateGallery onChoose={() => {}} />)
    expect(screen.getByRole("button", { name: /Blank workflow/i })).toBeTruthy()
    for (const p of WORKFLOW_PRESETS) {
      expect(
        screen.getByRole("button", { name: new RegExp(p.description) }),
      ).toBeTruthy()
    }
  })

  it("choosing a preset seeds its stages and blanks the name (named on Overview)", () => {
    const onChoose = vi.fn()
    render(<TemplateGallery onChoose={onChoose} />)
    fireEvent.click(
      screen.getByRole("button", {
        name: new RegExp(WORKFLOW_PRESETS[0].description),
      }),
    )
    const doc = onChoose.mock.calls[0][0] as WorkflowDocument
    expect(doc.stages.length).toBeGreaterThan(0)
    // The repo isn't chosen yet at gallery time, so the name is left blank
    // for the Overview tab's placeholder to guide — never a repo-derived
    // name with empty owner/name.
    expect(doc.name).toBe("")
  })

  it("choosing Blank yields an empty, step-less document", () => {
    const onChoose = vi.fn()
    render(<TemplateGallery onChoose={onChoose} />)
    fireEvent.click(screen.getByRole("button", { name: /Blank workflow/i }))
    const doc = onChoose.mock.calls[0][0] as WorkflowDocument
    expect(doc.stages.length).toBe(0)
    expect(doc.name).toBe("")
  })

  it("every preset builds a readable name when no repo has been picked yet", () => {
    // Regression: building the Issue → PR preset before the trigger repo was
    // picked used to produce names like "/ — issue to PR" (empty owner/name
    // around a literal slash). Every preset must fall back to a placeholder.
    const { trigger } = emptyDocument()
    for (const p of WORKFLOW_PRESETS) {
      const next = p.build(trigger)
      expect(next.name, `${p.id} name`).not.toMatch(/^\s*\//)
      expect(next.name, `${p.id} name`).not.toMatch(/\/\s+—/)
    }
  })
})
