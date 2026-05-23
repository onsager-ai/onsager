import type { ReactNode } from "react"
import { describe, it, expect, vi } from "vitest"
import { render, screen, fireEvent, act } from "@testing-library/react"
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { MemoryRouter } from "react-router-dom"
import {
  CommandPaletteProvider,
  CommandPaletteTrigger,
  parseAskCommand,
} from "@/components/layout/CommandPalette"
import { ChatUiProvider, useChatUi } from "@/lib/chat/use-chat-ui"

// Regression: clicking the search trigger used to render a blank
// browser because shadcn's CommandDialog does not auto-wrap children
// in <Command>, so cmdk primitives threw on missing context. This
// test mounts the trigger, clicks it, and asserts the palette renders
// real content (groups + items) rather than crashing the tree.
function mount(children?: ReactNode) {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return render(
    <QueryClientProvider client={qc}>
      <MemoryRouter>
        <ChatUiProvider>
          <CommandPaletteProvider>
            <CommandPaletteTrigger />
            {children}
          </CommandPaletteProvider>
        </ChatUiProvider>
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
    // Three-tab IA (ADR 0019): Workflows / Runs / Activity replace the
    // legacy `Chat` go-to entry.
    expect(screen.getByText(/^workflows$/i)).toBeInTheDocument()
    expect(screen.getByText(/^runs$/i)).toBeInTheDocument()
    expect(screen.getByText(/^activity$/i)).toBeInTheDocument()
    // Chat invocation (ADR 0020 channel 5, spec #473).
    expect(screen.getByText(/ask the assistant/i)).toBeInTheDocument()
  })
})

describe("parseAskCommand", () => {
  it("returns null for non-ask input", () => {
    expect(parseAskCommand("")).toBeNull()
    expect(parseAskCommand("workflows")).toBeNull()
    expect(parseAskCommand("askbar")).toBeNull()
  })

  it("returns empty string for the bare keyword", () => {
    expect(parseAskCommand("ask")).toBe("")
    expect(parseAskCommand("ASK")).toBe("")
    expect(parseAskCommand("  ask  ")).toBe("")
    expect(parseAskCommand("ask:")).toBe("")
  })

  it("extracts the inline message after the separator", () => {
    expect(parseAskCommand("ask: why did run #42 fail?")).toBe(
      "why did run #42 fail?",
    )
    expect(parseAskCommand("ask why did run #42 fail?")).toBe(
      "why did run #42 fail?",
    )
    expect(parseAskCommand("Ask: hello")).toBe("hello")
  })
})

// Spec #473 — the ⌘K palette opens the global chat at the current
// route's scope, default tab `Thread`, with the inline message
// (when present) routed through `useChatUi.open({ prefill })`.
describe("CommandPalette → chat invocation", () => {
  function ChatUiProbe({
    onState,
  }: {
    onState: (s: {
      isOpen: boolean
      tab: string
      prefill: string | null
      scopeOverride: unknown
    }) => void
  }) {
    const { isOpen, tab, prefill, scopeOverride } = useChatUi()
    onState({ isOpen, tab, prefill, scopeOverride })
    return null
  }

  it("ask + Enter opens the chat with no prefill on the Thread tab", async () => {
    const onState = vi.fn()
    mount(<ChatUiProbe onState={onState} />)

    fireEvent.click(screen.getByRole("button", { name: /command palette/i }))
    const input = screen.getByPlaceholderText(/type a command or search/i)
    fireEvent.change(input, { target: { value: "ask" } })

    await act(async () => {
      fireEvent.keyDown(input, { key: "Enter" })
    })

    const last = onState.mock.calls.at(-1)?.[0]
    expect(last.isOpen).toBe(true)
    expect(last.tab).toBe("thread")
    expect(last.prefill).toBeNull()
    // No scope override — auto-scope follows the current route.
    expect(last.scopeOverride).toBeNull()
  })

  it("`ask: <msg>` + Enter opens the chat with the message pre-filled", async () => {
    const onState = vi.fn()
    mount(<ChatUiProbe onState={onState} />)

    fireEvent.click(screen.getByRole("button", { name: /command palette/i }))
    const input = screen.getByPlaceholderText(/type a command or search/i)
    fireEvent.change(input, {
      target: { value: "ask: why did run #42 fail?" },
    })

    // The inline label confirms cmdk surfaces the chat item even
    // though the query wouldn't naturally score against "ask".
    expect(screen.getByText(/^Ask:$/)).toBeInTheDocument()
    expect(screen.getByText(/why did run #42 fail\?/)).toBeInTheDocument()

    await act(async () => {
      fireEvent.keyDown(input, { key: "Enter" })
    })

    const last = onState.mock.calls.at(-1)?.[0]
    expect(last.isOpen).toBe(true)
    expect(last.tab).toBe("thread")
    expect(last.prefill).toBe("why did run #42 fail?")
    expect(last.scopeOverride).toBeNull()
  })
})
