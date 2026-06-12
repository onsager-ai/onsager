import { describe, it, expect, vi, beforeEach } from "vitest"
import { render, screen, fireEvent, act } from "@testing-library/react"
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { MemoryRouter, Routes, Route } from "react-router-dom"

import { DesktopChatEntry } from "@/components/chat/DesktopChatEntry"
import { ChatBell } from "@/components/chat/ChatBell"
import { ChatContainer } from "@/components/chat/ChatContainer"
import { ChatUiProvider } from "@/lib/chat/use-chat-ui"
import { WorkspaceScope } from "@/lib/workspace"

// ADR 0025 / spec #514: the labeled desktop chat entry opens the
// conversation surface (Thread tab) at workspace scope — distinct from
// the bell, which stays the Inbox/Queue counter. Both are asserted here.

vi.mock("@/lib/auth", () => ({
  useAuth: () => ({ user: { id: "user-1", login: "tester" }, authEnabled: true }),
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
  vi.clearAllMocks()
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
      scope_type: "workspace",
      scope_id: null,
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
                  <DesktopChatEntry />
                  <ChatBell />
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

describe("DesktopChatEntry (spec #514)", () => {
  it("opens the chat at workspace scope on the Thread tab", async () => {
    mount()
    const entry = await screen.findByRole("button", { name: /open chat/i })
    await act(async () => {
      fireEvent.click(entry)
    })
    await screen.findByRole("dialog", { name: /onsager chat/i })

    const thread = screen.getByRole("tab", { name: /thread/i })
    expect(
      thread.getAttribute("data-state") ?? thread.getAttribute("aria-selected"),
    ).toMatch(/active|true/)

    await act(async () => {
      await new Promise((r) => setTimeout(r, 0))
    })
    expect(apiMock.getThread).toHaveBeenCalledWith("ws-1", {
      scope_type: "workspace",
    })
  })

  it("leaves the bell as the Inbox/Queue entry", async () => {
    mount()
    const bell = await screen.findByRole("button", { name: /open inbox/i })
    await act(async () => {
      fireEvent.click(bell)
    })
    await screen.findByRole("dialog", { name: /onsager chat/i })
    const queue = screen.getByRole("tab", { name: /queue/i })
    expect(
      queue.getAttribute("data-state") ?? queue.getAttribute("aria-selected"),
    ).toMatch(/active|true/)
  })
})
