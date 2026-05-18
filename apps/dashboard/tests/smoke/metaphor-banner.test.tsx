import { describe, it, expect, beforeEach } from "vitest"
import { render, screen, fireEvent } from "@testing-library/react"
import { MetaphorBanner } from "@/components/factory/workflows/MetaphorBanner"

const STORAGE_KEY = "onsager.metaphor_seen.workflow_detail"

describe("MetaphorBanner (#408 location 5)", () => {
  beforeEach(() => {
    window.localStorage.clear()
  })

  it("renders the locked copy on first visit", () => {
    render(<MetaphorBanner />)
    expect(
      screen.getByText(
        /This is your first production line\. Each stage is a work station; each gate is a QC checkpoint/,
      ),
    ).toBeTruthy()
  })

  it("hides after Got it is clicked and persists across remounts", () => {
    const { unmount } = render(<MetaphorBanner />)
    fireEvent.click(screen.getByRole("button", { name: /Dismiss/ }))
    expect(screen.queryByText(/your first production line/)).toBeNull()
    expect(window.localStorage.getItem(STORAGE_KEY)).toBe("1")

    unmount()
    render(<MetaphorBanner />)
    expect(screen.queryByText(/your first production line/)).toBeNull()
  })

  it("stays hidden when localStorage is already set", () => {
    window.localStorage.setItem(STORAGE_KEY, "1")
    render(<MetaphorBanner />)
    expect(screen.queryByText(/your first production line/)).toBeNull()
  })
})
