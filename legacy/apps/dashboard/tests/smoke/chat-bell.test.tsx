import { describe, it, expect, vi, beforeEach } from "vitest"
import { render, screen, fireEvent, act } from "@testing-library/react"
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { MemoryRouter, Routes, Route } from "react-router-dom"

import { ChatBell } from "@/components/chat/ChatBell"
import { ChatContainer } from "@/components/chat/ChatContainer"
import { ChatUiProvider } from "@/lib/chat/use-chat-ui"
import { WorkspaceScope } from "@/lib/workspace"
import { dispatchScopeHitlCountChange } from "@/lib/chat/use-hitl-count"
import { WORKSPACE_SCOPE } from "@/lib/chat/scope"

// ChatBell smoke coverage (spec #472 Plan: "E2E: bell opens chat at
// scope=workspace, Queue tab").

vi.mock("@/lib/auth", () => ({
  useAuth: () => ({
    user: { id: "user-1", login: "tester" },
    authEnabled: true,
  }),
}))

const apiMock = vi.hoisted(() => ({
  listWorkspaces: vi.fn(),
  getThread: vi.fn(),
}))

vi.mock("@/lib/api", async () => {
  const actual =
    await vi.importActual<typeof import("@/lib/api")>("@/lib/api")
  return {
    ...actual,
    api: { ...actual.api, listWorkspaces: apiMock.listWorkspaces },
  }
})

vi.mock("@/lib/api/chat", async () => {
  const actual = await vi.importActual<typeof import("@/lib/api/chat")>(
    "@/lib/api/chat",
  )
  return {
    ...actual,
    chat: {
      getThread: apiMock.getThread,
      appendMessage: vi.fn(),
      search: vi.fn(),
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
  apiMock.getThread.mockResolvedValue({
    thread: {
      id: "t-1",
      user_id: "user-1",
      workspace_id: "ws-1",
      scope_type: "workspace",
      scope_id: null,
      created_at: "",
      updated_at: "",
    },
    messages: [],
  })
  dispatchScopeHitlCountChange(WORKSPACE_SCOPE, 0)
})

function mount(initialPath = "/workspaces/acme/runs/run-42") {
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
                  <ChatBell />
                  <Routes>
                    <Route path="runs/:runId" element={<div />} />
                  </Routes>
                </WorkspaceScope>
              }
            />
          </Routes>
          <ChatContainer />
        </ChatUiProvider>
      </MemoryRouter>
    </QueryClientProvider>,
  )
}

describe("ChatBell", () => {
  it("renders without a badge when no pending HITL", async () => {
    mount()
    const bell = await screen.findByRole("button", { name: /open inbox/i })
    expect(bell.getAttribute("data-count")).toBe("0")
  })

  it("shows the workspace HITL count on the badge", async () => {
    mount()
    await screen.findByRole("button", { name: /open inbox/i })
    await act(async () => {
      dispatchScopeHitlCountChange(WORKSPACE_SCOPE, 7)
    })
    const bell = await screen.findByRole("button", {
      name: /open inbox — 7 pending/i,
    })
    expect(bell.getAttribute("data-count")).toBe("7")
    expect(screen.getByText("7")).toBeInTheDocument()
  })

  it("opens the chat at workspace scope on the Queue tab", async () => {
    // Even when the route is run detail, clicking the bell scopes to
    // workspace + Queue.
    mount("/workspaces/acme/runs/run-42")
    const bell = await screen.findByRole("button", { name: /open inbox/i })
    await act(async () => {
      fireEvent.click(bell)
    })
    const dialog = await screen.findByRole("dialog", { name: /onsager chat/i })
    expect(dialog).toBeInTheDocument()
    // Queue tab is active.
    const queue = screen.getByRole("tab", { name: /queue/i })
    expect(queue.getAttribute("data-state") ?? queue.getAttribute("aria-selected")).toMatch(/active|true/)
    // getThread fired with workspace scope, not run.
    await act(async () => {
      await new Promise((r) => setTimeout(r, 0))
    })
    expect(apiMock.getThread).toHaveBeenCalledWith("ws-1", {
      scope_type: "workspace",
    })
  })
})
