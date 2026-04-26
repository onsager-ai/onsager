import { describe, it, expect } from "vitest"
import { render, screen, fireEvent } from "@testing-library/react"
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { MemoryRouter } from "react-router-dom"
import {
  CommandPaletteProvider,
  CommandPaletteTrigger,
} from "@/components/layout/CommandPalette"

// Regression: clicking the search trigger used to render a blank
// browser because shadcn's CommandDialog does not auto-wrap children
// in <Command>, so cmdk primitives threw on missing context. This
// test mounts the trigger, clicks it, and asserts the palette renders
// real content (groups + items) rather than crashing the tree.
function mount() {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return render(
    <QueryClientProvider client={qc}>
      <MemoryRouter>
        <CommandPaletteProvider>
          <CommandPaletteTrigger />
        </CommandPaletteProvider>
      </MemoryRouter>
    </QueryClientProvider>,
  )
}

describe("CommandPalette", () => {
  it("renders the trigger button without throwing", () => {
    mount()
    expect(
      screen.getByRole("button", { name: /command palette/i }),
    ).toBeInTheDocument()
  })

  it("opens a populated palette when the trigger is clicked", () => {
    mount()
    fireEvent.click(screen.getByRole("button", { name: /command palette/i }))

    // The palette must render its searchable input + at least one item
    // from each group. If <Command> context is missing, cmdk primitives
    // throw and none of these would appear (blank screen).
    expect(
      screen.getByPlaceholderText(/type a command or search/i),
    ).toBeInTheDocument()
    expect(screen.getByText(/new workflow/i)).toBeInTheDocument()
    expect(screen.getByText(/factory overview/i)).toBeInTheDocument()
  })
})
