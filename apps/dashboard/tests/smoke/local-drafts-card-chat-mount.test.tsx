import { describe, it, expect, vi, beforeEach } from "vitest"
import { render, screen } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { MemoryRouter } from "react-router-dom"

import { LocalDraftsCard } from "@/components/workflows/LocalDraftsCard"
import { ChatUiProvider, useChatUi } from "@/lib/chat/use-chat-ui"

// `/chat` and `/workspaces/:slug/chat` were retired as page routes
// (#468 / ADR 0019). The local-drafts card's "Continue" and "New draft"
// affordances used to navigate to `/chat?draft=<id>`; the ADR 0020
// global chat mount means they now invoke `useChatUi().open()` instead.
// This pins that contract so a future regression to `navigate('/chat')`
// fails loudly rather than silently bouncing through a redirect.

vi.mock("@/lib/auth", () => ({
  useAuth: () => ({ user: null, authEnabled: false }),
}))

const ossMock = vi.hoisted(() => ({ useOSSFlag: vi.fn() }))
vi.mock("@/hooks/useOSSFlag", () => ossMock)

function seedDrafts(items: { id: string; name: string }[]) {
  const drafts = items.map((it) => ({
    id: it.id,
    user_id: "anon",
    name: it.name,
    source: "blank" as const,
    workflow: {
      name: "",
      trigger: { install_id: "", repo_owner: "", repo_name: "", label: "" },
      stages: [],
    },
    created_at: new Date().toISOString(),
    updated_at: new Date().toISOString(),
  }))
  window.localStorage.setItem(
    "onsager.drafts.anon",
    JSON.stringify({ drafts }),
  )
}

// Probe component reads `useChatUi().isOpen` and renders it as text so
// the test can assert the open/closed transition without depending on
// ChatContainer (which has its own provider needs for auth/workspaces).
function ChatOpenProbe() {
  const { isOpen } = useChatUi()
  return <div data-testid="chat-open">{isOpen ? "open" : "closed"}</div>
}

function renderCard() {
  return render(
    <MemoryRouter>
      <ChatUiProvider>
        <LocalDraftsCard />
        <ChatOpenProbe />
      </ChatUiProvider>
    </MemoryRouter>,
  )
}

beforeEach(() => {
  window.localStorage.clear()
  ossMock.useOSSFlag.mockReturnValue(false)
})

describe("LocalDraftsCard — chat-mount invocation (#468)", () => {
  it("clicking `Continue` on a draft row opens the global chat mount", async () => {
    seedDrafts([{ id: "d_1", name: "My draft" }])
    const user = userEvent.setup()
    renderCard()
    expect(screen.getByTestId("chat-open").textContent).toBe("closed")
    // The row has two clickable elements — the row tile and the
    // explicit "Continue" button. Both should open the chat mount;
    // we click the explicit button as the user-visible affordance.
    const continueBtn = await screen.findByRole("button", { name: /^Continue$/ })
    await user.click(continueBtn)
    expect(screen.getByTestId("chat-open").textContent).toBe("open")
  })

  it("clicking the row tile also opens the global chat mount", async () => {
    seedDrafts([{ id: "d_1", name: "My draft" }])
    const user = userEvent.setup()
    renderCard()
    expect(screen.getByTestId("chat-open").textContent).toBe("closed")
    const tile = await screen.findByRole("button", {
      name: /Continue draft My draft in chat/i,
    })
    await user.click(tile)
    expect(screen.getByTestId("chat-open").textContent).toBe("open")
  })

  it("clicking `New draft` opens the chat mount on a fresh draft", async () => {
    const user = userEvent.setup()
    renderCard()
    expect(screen.getByTestId("chat-open").textContent).toBe("closed")
    const newBtn = await screen.findByRole("button", { name: /New draft/i })
    await user.click(newBtn)
    expect(screen.getByTestId("chat-open").textContent).toBe("open")
  })
})
