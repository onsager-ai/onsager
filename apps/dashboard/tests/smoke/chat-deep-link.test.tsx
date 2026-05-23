import { describe, it, expect, vi, beforeEach } from "vitest"
import { render, screen, waitFor } from "@testing-library/react"
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { MemoryRouter, Routes, Route } from "react-router-dom"

import { ChatDeepLinkHandler } from "@/components/chat/ChatDeepLinkHandler"
import { ChatContainer } from "@/components/chat/ChatContainer"
import { ChatUiProvider } from "@/lib/chat/use-chat-ui"
import { WorkspaceScope } from "@/lib/workspace"

// Deep-link handler smoke coverage (spec #472 Plan: "Deep-link
// handler: push → chat at the HITL's scope, Queue tab, card
// highlighted"). The service worker shapes the click URL with
// `?onsager_chat=open&onsager_chat_scope=...&onsager_hitl_id=...`.

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
      scope_type: "run",
      scope_id: "run-42",
      created_at: "",
      updated_at: "",
    },
    messages: [],
  })
})

function mount(initialPath: string) {
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
                  <ChatDeepLinkHandler />
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

describe("ChatDeepLinkHandler", () => {
  it("opens chat at the carried scope + Queue tab + highlighted HITL", async () => {
    mount(
      "/workspaces/acme/runs/run-42?onsager_chat=open&onsager_chat_scope=run:run-42&onsager_hitl_id=h-1",
    )
    const dialog = await screen.findByRole("dialog", { name: /onsager chat/i })
    expect(dialog).toBeInTheDocument()
    // Queue tab is selected.
    const queue = screen.getByRole("tab", { name: /queue/i })
    expect(queue.getAttribute("data-state") ?? queue.getAttribute("aria-selected")).toMatch(/active|true/)

    // The highlight id propagated to the queue view.
    const empty = await screen.findByText(/no pending decisions in this scope/i)
    expect(empty.parentElement?.getAttribute("data-highlight-hitl-id")).toBe("h-1")

    // Wait until the Thread query fires with the run scope from the
    // deep link. ChatThreadView is `keepMounted` so it warms in the
    // background even on the Queue tab, but it still needs the
    // listWorkspaces query to resolve first.
    await waitFor(() =>
      expect(apiMock.getThread).toHaveBeenCalledWith("ws-1", {
        scope_type: "run",
        scope_id: "run-42",
      }),
    )
  })
})
