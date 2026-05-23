import { describe, it, expect, vi, beforeEach } from "vitest"
import { render, screen, fireEvent, act } from "@testing-library/react"
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { MemoryRouter, Routes, Route } from "react-router-dom"

import { ChatAskButton } from "@/components/chat/ChatAskButton"
import { ChatContainer } from "@/components/chat/ChatContainer"
import { ChatUiProvider } from "@/lib/chat/use-chat-ui"
import { WorkspaceScope } from "@/lib/workspace"

// ChatAskButton smoke coverage (spec #472 Plan: "E2E: scope-local Ask
// opens at the right object-level scope, Thread tab"). The route here
// is workflows-list (workspace scope by auto-derivation); Ask overrides
// it with a `verdict:{id}` scope, so the chat opens against verdict
// not workspace.

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

beforeEach(() => {
  apiMock.listWorkspaces.mockResolvedValue({
    workspaces: [
      {
        id: "ws-1",
        slug: "acme",
        name: "Acme",
        account: "acme",
        account_type: "organization" as const,
      },
    ],
  })
  apiMock.getThread.mockResolvedValue({
    thread: {
      id: "t-1",
      user_id: "user-1",
      workspace_id: "ws-1",
      scope_type: "verdict",
      scope_id: "vd-1",
      created_at: "",
      updated_at: "",
    },
    messages: [],
  })
})

function mount() {
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  return render(
    <QueryClientProvider client={qc}>
      <MemoryRouter initialEntries={["/workspaces/acme/workflows"]}>
        <ChatUiProvider>
          <Routes>
            <Route
              path="/workspaces/:workspace/*"
              element={
                <WorkspaceScope>
                  <ChatAskButton scope={{ type: "verdict", id: "vd-1" }} />
                  <Routes>
                    <Route path="workflows" element={<div />} />
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

describe("ChatAskButton", () => {
  it("opens the chat at the given object-level scope on the Thread tab", async () => {
    mount()
    const ask = await screen.findByRole("button", { name: /ask/i })
    expect(ask.getAttribute("data-scope-type")).toBe("verdict")

    await act(async () => {
      fireEvent.click(ask)
    })
    await screen.findByRole("dialog", { name: /onsager chat/i })

    // Thread tab is the default for Ask.
    const thread = screen.getByRole("tab", { name: /thread/i })
    expect(thread.getAttribute("data-state") ?? thread.getAttribute("aria-selected")).toMatch(/active|true/)

    await act(async () => {
      await new Promise((r) => setTimeout(r, 0))
    })
    // Override prevailed over the route-derived workspace scope.
    expect(apiMock.getThread).toHaveBeenCalledWith("ws-1", {
      scope_type: "verdict",
      scope_id: "vd-1",
    })
  })
})
