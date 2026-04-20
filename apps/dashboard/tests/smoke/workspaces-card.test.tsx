import { describe, it, expect, vi, beforeEach } from "vitest"
import { render, screen, fireEvent, waitFor } from "@testing-library/react"
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { MemoryRouter } from "react-router-dom"
import { WorkspaceCard } from "@/components/workspaces/WorkspaceCard"

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

async function primeMocks(opts: {
  repos: unknown[]
  installations?: unknown[]
  projects?: unknown[]
  appEnabled?: boolean
  members?: unknown[]
}) {
  const { api } = await import("@/lib/api")
  vi.mocked(api.listWorkspaces).mockResolvedValue({ tenants: [ws] })
  vi.mocked(api.listWorkspaceMembers).mockResolvedValue({
    members: (opts.members ?? []) as never,
  })
  vi.mocked(api.listWorkspaceInstallations).mockResolvedValue({
    installations: (opts.installations ?? [orgInstall]) as never,
  })
  vi.mocked(api.listWorkspaceProjects).mockResolvedValue({
    projects: (opts.projects ?? []) as never,
  })
  vi.mocked(api.listInstallationRepos).mockResolvedValue({
    repos: opts.repos as never,
  })
  vi.mocked(api.getGitHubAppConfig).mockResolvedValue({
    enabled: opts.appEnabled ?? true,
  })
}

function renderCard() {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return render(
    <MemoryRouter>
      <QueryClientProvider client={qc}>
        <WorkspaceCard workspace={ws} />
      </QueryClientProvider>
    </MemoryRouter>,
  )
}

describe("WorkspaceCard — OAuth-first Add project flow", () => {
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
    await screen.findByRole("button", { name: /add another installation/i })
    expect(
      screen.queryByRole("button", { name: /link manually/i }),
    ).not.toBeInTheDocument()
    expect(
      screen.queryByPlaceholderText(/installation id/i),
    ).not.toBeInTheDocument()
  })

  it("tells the user to contact an admin when the GitHub App is not configured", async () => {
    await primeMocks({ repos: [], installations: [], appEnabled: false })
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
      screen.queryByRole("button", { name: /add another installation/i }),
    ).not.toBeInTheDocument()
    expect(
      screen.queryByRole("button", { name: /link manually/i }),
    ).not.toBeInTheDocument()
  })
})

describe("WorkspaceCard — step-by-step NextStepCallout", () => {
  beforeEach(() => vi.clearAllMocks())

  it("shows a Connect GitHub CTA when no installations are linked yet", async () => {
    await primeMocks({ repos: [], installations: [] })
    renderCard()
    const link = await screen.findByRole("link", { name: /install github app/i })
    expect(link.getAttribute("href")).toBe(
      "/api/github-app/install-start?tenant_id=ws1",
    )
    expect(screen.getByText(/step 2 of 3/i)).toBeInTheDocument()
    // Empty-state CTA must be unambiguous — the InstallationsSection
    // must not duplicate the callout's install button.
    expect(
      screen.queryByRole("button", { name: /install via github app/i }),
    ).not.toBeInTheDocument()
    expect(
      screen.queryByRole("button", { name: /add another installation/i }),
    ).not.toBeInTheDocument()
  })

  it("blocks setup with an admin-action callout when the GitHub App is unavailable", async () => {
    await primeMocks({ repos: [], installations: [], appEnabled: false })
    renderCard()
    expect(
      await screen.findByText(/setup blocked: github app unavailable/i),
    ).toBeInTheDocument()
    expect(
      screen.queryByRole("link", { name: /install github app/i }),
    ).not.toBeInTheDocument()
  })

  it("shows an Add project CTA once GitHub is connected but no project is linked", async () => {
    await primeMocks({
      repos: [
        { owner: "onsager-ai", name: "onsager", default_branch: "main", private: false },
      ],
    })
    renderCard()
    const cta = await screen.findByRole("button", { name: /^add project/i })
    expect(cta).toBeInTheDocument()
    expect(screen.getByText(/step 3 of 3/i)).toBeInTheDocument()
    // The empty-state button in the section is suppressed; only the callout
    // CTA drives the user forward.
    expect(
      screen.queryByRole("button", { name: /add another project/i }),
    ).not.toBeInTheDocument()
  })

  it("clicking the Add project CTA opens the add-project form", async () => {
    await primeMocks({
      repos: [
        { owner: "onsager-ai", name: "onsager", default_branch: "main", private: false },
      ],
    })
    renderCard()
    const cta = await screen.findByRole("button", { name: /^add project/i })
    fireEvent.click(cta)
    await waitFor(() =>
      expect(screen.getByTestId("add-project-form")).toBeInTheDocument(),
    )
    expect(
      await screen.findByRole("button", { name: /select a repository/i }),
    ).toBeInTheDocument()
  })

  it("invites the user to start a session once setup is complete", async () => {
    await primeMocks({
      repos: [],
      projects: [
        {
          id: "p1",
          tenant_id: "ws1",
          github_app_installation_id: "inst1",
          repo_owner: "onsager-ai",
          repo_name: "onsager",
          default_branch: "main",
          created_at: "2026-01-01",
        },
      ],
    })
    renderCard()
    const link = await screen.findByRole("link", { name: /start a session/i })
    expect(link.getAttribute("href")).toBe("/sessions")
    expect(screen.getByText(/you're set up/i)).toBeInTheDocument()
    // "Add another project" appears now that there's an existing project.
    expect(
      screen.getByRole("button", { name: /add another project/i }),
    ).toBeInTheDocument()
  })
})

describe("WorkspaceCard — human-readable member + installation labels", () => {
  beforeEach(() => vi.clearAllMocks())

  it("renders members as @login links to GitHub, not raw user UUIDs", async () => {
    await primeMocks({
      repos: [],
      members: [
        {
          tenant_id: "ws1",
          user_id: "133d228a-be10-492b-8035-bdb984d20721",
          joined_at: "2026-01-01",
          github_login: "octocat",
          github_name: "Octo Cat",
          github_avatar_url: "https://example.test/octocat.png",
        },
      ],
    })
    renderCard()
    const link = await screen.findByRole("link", { name: /@octocat/i })
    expect(link.getAttribute("href")).toBe("https://github.com/octocat")
    expect(link.getAttribute("target")).toBe("_blank")
    expect(
      screen.queryByText(/133d228a-be10-492b-8035-bdb984d20721/),
    ).not.toBeInTheDocument()
  })

  it("falls back to user_id when the joined users row has no github profile", async () => {
    await primeMocks({
      repos: [],
      members: [
        {
          tenant_id: "ws1",
          user_id: "orphaned-user-id",
          joined_at: "2026-01-01",
          github_login: null,
          github_name: null,
          github_avatar_url: null,
        },
      ],
    })
    renderCard()
    // No @login → no link; chip renders a plain span with the raw user_id
    // so operators can still see something instead of an empty box.
    await screen.findByText(/orphaned-user-id/)
    expect(
      screen.queryByRole("link", { name: /orphaned-user-id/ }),
    ).not.toBeInTheDocument()
  })

  it("shows the installation account_login in the Select trigger, not the UUID", async () => {
    await primeMocks({
      repos: [
        { owner: "onsager-ai", name: "onsager", default_branch: "main", private: false },
      ],
    })
    const { container } = renderCard()
    fireEvent.click(await screen.findByRole("button", { name: /add project/i }))
    await screen.findByTestId("add-project-form")
    // Grab the SelectValue span inside the Select trigger (the [data-slot]
    // attribute is stamped by the shadcn wrapper). It must render the
    // installation's account_login after the form auto-selects the first
    // installation, not the raw installation UUID `inst1`.
    await waitFor(() => {
      const selectValue = container.querySelector('[data-slot="select-value"]')
      expect(selectValue?.textContent).toMatch(/onsager-ai/i)
      expect(selectValue?.textContent).not.toMatch(/inst1/)
    })
  })
})
