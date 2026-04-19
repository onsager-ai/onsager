import { describe, it, expect, vi, beforeEach } from "vitest"
import { render, screen, fireEvent, waitFor } from "@testing-library/react"
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { WorkspacesCard } from "@/components/settings/WorkspacesCard"

vi.mock("@/lib/api", () => ({
  api: {
    listWorkspaces: vi.fn(),
    listWorkspaceMembers: vi.fn(),
    listWorkspaceInstallations: vi.fn(),
    listWorkspaceProjects: vi.fn(),
    listInstallationRepos: vi.fn(),
    getGitHubAppConfig: vi.fn(),
    registerWorkspaceInstallation: vi.fn(),
    deleteWorkspaceInstallation: vi.fn(),
    addWorkspaceProject: vi.fn(),
    deleteProject: vi.fn(),
    createWorkspace: vi.fn(),
  },
}))

const ws = {
  id: "ws1",
  slug: "acme",
  name: "Acme",
  created_by: "u1",
  created_at: "2026-01-01",
}
const orgInstall = {
  id: "inst1",
  tenant_id: "ws1",
  install_id: 42,
  account_login: "onsager-ai",
  account_type: "organization" as const,
  created_at: "2026-01-01",
}

async function primeMocks(opts: { repos: unknown[] }) {
  const { api } = await import("@/lib/api")
  vi.mocked(api.listWorkspaces).mockResolvedValue({ tenants: [ws] })
  vi.mocked(api.listWorkspaceMembers).mockResolvedValue({ members: [] })
  vi.mocked(api.listWorkspaceInstallations).mockResolvedValue({
    installations: [orgInstall],
  })
  vi.mocked(api.listWorkspaceProjects).mockResolvedValue({ projects: [] })
  vi.mocked(api.listInstallationRepos).mockResolvedValue({
    repos: opts.repos as never,
  })
  vi.mocked(api.getGitHubAppConfig).mockResolvedValue({ enabled: true })
}

function renderCard() {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return render(
    <QueryClientProvider client={qc}>
      <WorkspacesCard />
    </QueryClientProvider>,
  )
}

describe("WorkspacesCard — OAuth-first Add project flow", () => {
  beforeEach(() => vi.clearAllMocks())

  it("deep-links to GitHub install settings when the install has no accessible repos", async () => {
    await primeMocks({ repos: [] })
    renderCard()
    const addBtn = await screen.findByRole("button", { name: /add project/i })
    fireEvent.click(addBtn)
    const link = await screen.findByRole("link", {
      name: /configure repository access on github/i,
    })
    expect(link.getAttribute("href")).toBe(
      "https://github.com/organizations/onsager-ai/settings/installations/42",
    )
    expect(link.getAttribute("target")).toBe("_blank")
  })

  it("does not render any manual owner/name Input fields in the add form", async () => {
    await primeMocks({
      repos: [
        { owner: "onsager-ai", name: "onsager", default_branch: "main", private: false },
      ],
    })
    renderCard()
    const addBtn = await screen.findByRole("button", { name: /add project/i })
    fireEvent.click(addBtn)
    await waitFor(() =>
      expect(
        screen.queryByPlaceholderText(/repo owner/i),
      ).not.toBeInTheDocument(),
    )
    expect(screen.queryByPlaceholderText(/^repo name$/i)).not.toBeInTheDocument()
    expect(
      screen.queryByPlaceholderText(/github\.com\/owner\/repo/i),
    ).not.toBeInTheDocument()
  })

  it("uses a search+select picker for the repo (not free-form typing)", async () => {
    await primeMocks({
      repos: [
        { owner: "onsager-ai", name: "onsager", default_branch: "main", private: false },
        { owner: "onsager-ai", name: "onsager-infra", default_branch: "main", private: true },
      ],
    })
    renderCard()
    fireEvent.click(await screen.findByRole("button", { name: /add project/i }))
    const trigger = await screen.findByRole("button", {
      name: /select a repository/i,
    })
    expect(trigger).toBeInTheDocument()
    expect(trigger.getAttribute("aria-expanded")).toBe("false")
  })

  it("never renders a 'Link manually' button", async () => {
    await primeMocks({ repos: [] })
    renderCard()
    await screen.findByRole("button", { name: /install via github app/i })
    expect(
      screen.queryByRole("button", { name: /link manually/i }),
    ).not.toBeInTheDocument()
    expect(
      screen.queryByPlaceholderText(/installation id/i),
    ).not.toBeInTheDocument()
  })

  it("tells the user to contact an admin when the GitHub App is not configured", async () => {
    const { api } = await import("@/lib/api")
    vi.mocked(api.listWorkspaces).mockResolvedValue({ tenants: [ws] })
    vi.mocked(api.listWorkspaceMembers).mockResolvedValue({ members: [] })
    vi.mocked(api.listWorkspaceInstallations).mockResolvedValue({
      installations: [],
    })
    vi.mocked(api.listWorkspaceProjects).mockResolvedValue({ projects: [] })
    vi.mocked(api.getGitHubAppConfig).mockResolvedValue({ enabled: false })
    renderCard()
    await waitFor(() =>
      expect(
        screen.getByText(/github app is not configured/i),
      ).toBeInTheDocument(),
    )
    expect(
      screen.queryByRole("button", { name: /install via github app/i }),
    ).not.toBeInTheDocument()
    expect(
      screen.queryByRole("button", { name: /link manually/i }),
    ).not.toBeInTheDocument()
  })
})
