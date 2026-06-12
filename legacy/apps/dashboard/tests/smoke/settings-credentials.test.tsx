import { describe, it, expect, vi, beforeEach } from "vitest"
import { render, screen, fireEvent, waitFor } from "@testing-library/react"
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { MemoryRouter, Routes, Route } from "react-router-dom"
import { WorkspaceSettingsPage } from "@/pages/WorkspaceSettingsPage"

// Per spec #166, credentials moved from the account-wide /settings page
// to /workspaces/:workspace/settings. The dialog wires through the active
// workspace via WorkspaceContext, so the smoke tests render under the
// scoped layout.

vi.mock("@/lib/auth", () => ({
  useAuth: () => ({ user: null, authEnabled: false }),
}))

vi.mock("@/lib/api", () => ({
  api: {
    listWorkspaces: vi
      .fn()
      .mockResolvedValue({
        workspaces: [
          {
            id: "ws1",
            slug: "acme",
            name: "Acme",
            created_by: "u1",
            created_at: "2026-01-01",
          },
        ],
      }),
    getCredentials: vi.fn().mockResolvedValue({ credentials: [] }),
    setCredential: vi.fn(),
    deleteCredential: vi.fn(),
  },
}))

import { WorkspaceScope } from "@/lib/workspace"

function renderSettings() {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return render(
    <QueryClientProvider client={qc}>
      <MemoryRouter initialEntries={["/workspaces/acme/settings"]}>
        <Routes>
          <Route
            path="/workspaces/:workspace/*"
            element={
              <WorkspaceScope>
                <Routes>
                  <Route path="settings" element={<WorkspaceSettingsPage />} />
                </Routes>
              </WorkspaceScope>
            }
          />
        </Routes>
      </MemoryRouter>
    </QueryClientProvider>,
  )
}

describe("WorkspaceSettingsPage credentials layout", () => {
  beforeEach(() => vi.clearAllMocks())

  it("renders both known credential entries", async () => {
    renderSettings()
    expect(
      await screen.findByText("CLAUDE_CODE_OAUTH_TOKEN"),
    ).toBeInTheDocument()
    expect(screen.getByText("ANTHROPIC_API_KEY")).toBeInTheDocument()
  })

  it("shows Add button inline with each known credential (not in a separate row)", async () => {
    renderSettings()
    await screen.findByText("CLAUDE_CODE_OAUTH_TOKEN")
    const addButtons = screen.getAllByRole("button", { name: /add/i })
    // At least 2 Add buttons (one per known credential) + 1 for custom credential
    expect(addButtons.length).toBeGreaterThanOrEqual(2)
  })

  it("known credential name has truncate class to prevent mobile overflow", async () => {
    renderSettings()
    const token = await screen.findByText("CLAUDE_CODE_OAUTH_TOKEN")
    expect(token.className).toMatch(/truncate/)
  })

  it("reveals inline input form when Add is clicked on a known credential", async () => {
    renderSettings()
    await screen.findByText("CLAUDE_CODE_OAUTH_TOKEN")
    const [firstAdd] = screen.getAllByRole("button", { name: /add/i })
    fireEvent.click(firstAdd)
    expect(screen.getAllByPlaceholderText("Value").length).toBeGreaterThanOrEqual(1)
    expect(screen.getByRole("button", { name: /save/i })).toBeInTheDocument()
    expect(screen.getByRole("button", { name: /cancel/i })).toBeInTheDocument()
  })

  it("hides Add button while inline edit form is open", async () => {
    renderSettings()
    await screen.findByText("CLAUDE_CODE_OAUTH_TOKEN")
    const addButtons = screen.getAllByRole("button", { name: /add/i })
    const initialCount = addButtons.length
    fireEvent.click(addButtons[0])
    expect(screen.getAllByRole("button", { name: /add/i }).length).toBe(
      initialCount - 1,
    )
  })

  it("renders custom credential form with ENV_VAR_NAME and Value inputs", async () => {
    renderSettings()
    expect(
      await screen.findByPlaceholderText("ENV_VAR_NAME"),
    ).toBeInTheDocument()
    const valueInputs = screen.getAllByPlaceholderText("Value")
    expect(valueInputs.length).toBeGreaterThanOrEqual(1)
  })

  it("delete button has aria-label naming the credential", async () => {
    const { api: mockApi } = await import("@/lib/api")
    vi.mocked(mockApi.getCredentials).mockResolvedValueOnce({
      credentials: [{ name: "MY_SECRET", created_at: "x", updated_at: new Date().toISOString() }],
    })
    const { findByRole } = renderSettings()
    const deleteBtn = await findByRole("button", { name: "Delete MY_SECRET" })
    expect(deleteBtn).toBeInTheDocument()
  })

  it("inline edit form wraps inputs in a <form> element", async () => {
    renderSettings()
    await screen.findByText("CLAUDE_CODE_OAUTH_TOKEN")
    const [firstAdd] = screen.getAllByRole("button", { name: /add/i })
    fireEvent.click(firstAdd)
    const saveBtn = screen.getByRole("button", { name: /save/i })
    expect(saveBtn.closest("form")).not.toBeNull()
  })

  it("submitting inline edit form via Enter key triggers mutation", async () => {
    const { api: mockApi } = await import("@/lib/api")
    vi.mocked(mockApi.setCredential).mockResolvedValue({ ok: true })
    renderSettings()
    await screen.findByText("CLAUDE_CODE_OAUTH_TOKEN")
    const [firstAdd] = screen.getAllByRole("button", { name: /add/i })
    fireEvent.click(firstAdd)
    const input = screen.getAllByPlaceholderText(/value/i)[0]
    fireEvent.change(input, { target: { value: "secret123" } })
    fireEvent.submit(input.closest("form")!)
    await waitFor(() =>
      expect(mockApi.setCredential).toHaveBeenCalledWith(
        "ws1",
        "CLAUDE_CODE_OAUTH_TOKEN",
        "secret123",
      ),
    )
  })

  it("shows save error message when mutation fails", async () => {
    const { api: mockApi } = await import("@/lib/api")
    vi.mocked(mockApi.setCredential).mockRejectedValueOnce(
      new Error("Unauthorized"),
    )
    renderSettings()
    await screen.findByText("CLAUDE_CODE_OAUTH_TOKEN")
    const [firstAdd] = screen.getAllByRole("button", { name: /add/i })
    fireEvent.click(firstAdd)
    const input = screen.getAllByPlaceholderText(/value/i)[0]
    fireEvent.change(input, { target: { value: "bad" } })
    fireEvent.submit(input.closest("form")!)
    await waitFor(() =>
      expect(screen.getByText("Unauthorized")).toBeInTheDocument(),
    )
  })

  it("cancel clears the save error", async () => {
    const { api: mockApi } = await import("@/lib/api")
    vi.mocked(mockApi.setCredential).mockRejectedValueOnce(
      new Error("Unauthorized"),
    )
    renderSettings()
    await screen.findByText("CLAUDE_CODE_OAUTH_TOKEN")
    const [firstAdd] = screen.getAllByRole("button", { name: /add/i })
    fireEvent.click(firstAdd)
    const input = screen.getAllByPlaceholderText(/value/i)[0]
    fireEvent.change(input, { target: { value: "bad" } })
    fireEvent.submit(input.closest("form")!)
    await waitFor(() =>
      expect(screen.getByText("Unauthorized")).toBeInTheDocument(),
    )
    const [nextAdd] = screen.getAllByRole("button", { name: /add/i })
    fireEvent.click(nextAdd)
    fireEvent.click(screen.getByRole("button", { name: /cancel/i }))
    expect(screen.queryByText("Unauthorized")).toBeNull()
  })
})
