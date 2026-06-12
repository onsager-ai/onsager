import { describe, it, expect, vi, beforeEach, afterEach } from "vitest"
import { render, screen, fireEvent, act } from "@testing-library/react"
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { MemoryRouter, Routes, Route } from "react-router-dom"

import { ChatFab } from "@/components/chat/ChatFab"
import { ChatUiProvider, useChatUi } from "@/lib/chat/use-chat-ui"
import { WorkspaceScope } from "@/lib/workspace"
import { dispatchScopeHitlCountChange } from "@/lib/chat/use-hitl-count"
import { WORKSPACE_SCOPE } from "@/lib/chat/scope"

// ChatFab smoke coverage (spec #472 Plan: "Component tests for FAB
// three states + transitions" / "FAB long-press jumps to Inbox" /
// "first-time hint surfaces on third use"). jsdom doesn't resolve `md:`
// media queries so the FAB renders unconditionally — sufficient for
// state + interaction assertions.

vi.mock("@/lib/auth", () => ({
  useAuth: () => ({
    user: { id: "user-1", login: "tester" },
    authEnabled: true,
  }),
}))

const apiMock = vi.hoisted(() => ({
  listWorkspaces: vi.fn(),
}))

vi.mock("@/lib/api", async () => {
  const actual =
    await vi.importActual<typeof import("@/lib/api")>("@/lib/api")
  return {
    ...actual,
    api: {
      ...actual.api,
      listWorkspaces: apiMock.listWorkspaces,
    },
  }
})

const workspaceRow = {
  id: "ws-1",
  slug: "acme",
  name: "Acme",
  account: "acme",
  account_type: "organization" as const,
}

beforeEach(() => {
  window.localStorage.clear()
  apiMock.listWorkspaces.mockResolvedValue({ workspaces: [workspaceRow] })
  // Reset HITL counts between tests.
  dispatchScopeHitlCountChange(WORKSPACE_SCOPE, 0)
})

afterEach(() => {
  vi.useRealTimers()
})

function ChatUiSpy({ onChange }: { onChange: (open: boolean) => void }) {
  const { isOpen } = useChatUi()
  // Use a side-effect spy: the parent provider reflects open() calls
  // back; we re-render on isOpen change.
  onChange(isOpen)
  return null
}

function mountAt(initialPath: string, onUiChange = () => {}) {
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  return render(
    <QueryClientProvider client={qc}>
      <MemoryRouter initialEntries={[initialPath]}>
        <ChatUiProvider>
          <Routes>
            <Route
              path="/workspaces/:workspace/*"
              element={
                <WorkspaceScope>
                  <ChatUiSpy onChange={onUiChange} />
                  <Routes>
                    <Route path="workflows" element={<div />} />
                    <Route path="runs/:runId" element={<div />} />
                  </Routes>
                </WorkspaceScope>
              }
            />
          </Routes>
          <ChatFab />
        </ChatUiProvider>
      </MemoryRouter>
    </QueryClientProvider>,
  )
}

describe("ChatFab", () => {
  it("renders in the Muted state when no pending HITL", async () => {
    mountAt("/workspaces/acme/workflows")
    const fab = await screen.findByRole("button", { name: /open chat/i })
    expect(fab.getAttribute("data-state")).toBe("muted")
  })

  it("switches to Active when HITL arrives in the current scope", async () => {
    mountAt("/workspaces/acme/workflows")
    await screen.findByRole("button", { name: /open chat/i })
    await act(async () => {
      dispatchScopeHitlCountChange(WORKSPACE_SCOPE, 3)
    })
    const fab = await screen.findByRole("button", {
      name: /open chat — 3 pending/i,
    })
    expect(fab.getAttribute("data-state")).toBe("active")
    expect(screen.getByText("3")).toBeInTheDocument()
  })

  it("click opens chat with current-scope default tab", async () => {
    let isOpen = false
    mountAt("/workspaces/acme/workflows", (v) => (isOpen = v))
    const fab = await screen.findByRole("button", { name: /open chat/i })
    await act(async () => {
      fireEvent.click(fab)
    })
    expect(isOpen).toBe(true)
  })

  it("long-press fires after 500ms and opens at workspace scope", async () => {
    // Use real timers and a small actual sleep instead of fake timers —
    // act() + fake timers + React renders don't cooperate cleanly here,
    // and 500ms is still well under the test timeout.
    let isOpen = false
    mountAt("/workspaces/acme/runs/run-42", (v) => (isOpen = v))
    const fab = await screen.findByRole("button", { name: /open chat/i })

    await act(async () => {
      fireEvent.pointerDown(fab)
      await new Promise((r) => setTimeout(r, 550))
    })
    expect(isOpen).toBe(true)
  })

  it("surfaces the first-time hint on the third FAB use", async () => {
    // The FAB unmounts when the chat opens; we tap, programmatically
    // close, and tap again to reach the third use. The hint container
    // lives outside the open-gated button so it stays visible after
    // the third tap even when the chat is open.
    window.localStorage.setItem("onsager.chat.fab.useCount.v1", "2")
    mountAt("/workspaces/acme/workflows")
    const fab = await screen.findByRole("button", { name: /open chat/i })
    await act(async () => {
      fireEvent.click(fab)
    })
    expect(
      screen.getByText(/hold to jump to the workspace inbox/i),
    ).toBeInTheDocument()
    expect(window.localStorage.getItem("onsager.chat.fab.hintSeen.v1")).toBe("1")
  })

  it("does not surface the hint a second time once seen", async () => {
    window.localStorage.setItem("onsager.chat.fab.hintSeen.v1", "1")
    window.localStorage.setItem("onsager.chat.fab.useCount.v1", "2")
    mountAt("/workspaces/acme/workflows")
    const fab = await screen.findByRole("button", { name: /open chat/i })
    await act(async () => {
      fireEvent.click(fab)
    })
    expect(
      screen.queryByText(/hold to jump to the workspace inbox/i),
    ).toBeNull()
  })
})
