import { describe, it, expect, vi, beforeEach } from "vitest"
import { render, screen, fireEvent, waitFor } from "@testing-library/react"
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { MemoryRouter } from "react-router-dom"
import type { ReactNode } from "react"

import { BindDraftDialog } from "@/components/chat/BindDraftDialog"
import { makeDraft, saveDraft } from "@/lib/drafts"
import { makeStage } from "@/components/factory/workflows/workflow-draft"
import type {
  GitHubAppInstallation,
  Workspace,
} from "@/lib/api"

// ─── Fixtures ────────────────────────────────────────────────────────────────

const workspaces: Workspace[] = [
  {
    id: "ws_1",
    slug: "acme",
    name: "Acme",
    created_by: "u_1",
    created_at: "2026-01-01T00:00:00Z",
  },
]

const installations: GitHubAppInstallation[] = [
  {
    id: "inst_one",
    workspace_id: "ws_1",
    install_id: 1001,
    account_login: "onsager-ai",
    account_type: "organization",
    created_at: "2026-01-01T00:00:00Z",
  },
]

// One stage so the draft is "bindable" (≥ 1 stage). The trigger label is
// already set on the draft so `documentToCreateRequest` doesn't reject
// the bind in Step C tests. The draft is persisted via `saveDraft` so
// `markDraftBound` (which reads from localStorage) can find it during
// the Step C bind.
function makeBindableDraft() {
  const draft = makeDraft("u_1", "chat", {
    name: "Auto-merge on green",
    trigger: {
      install_id: "",
      repo_owner: "",
      repo_name: "",
      label: "auto-merge",
    },
    stages: [makeStage("agent-session", "Issue", "Spec → PR")],
  })
  saveDraft("u_1", draft)
  return draft
}

// ─── API mock ────────────────────────────────────────────────────────────────

vi.mock("@/lib/api", async () => {
  const actual = await vi.importActual<object>("@/lib/api")
  return {
    ...actual,
    api: {
      listWorkspaces: vi.fn(),
      listWorkspaceInstallations: vi.fn(),
      listWorkspaceProjects: vi.fn(),
      listInstallationRepos: vi.fn(),
      addWorkspaceProject: vi.fn(),
      createWorkflow: vi.fn(),
      createWorkspace: vi.fn(),
    },
  }
})

async function primeApi(opts: {
  workspaces?: Workspace[]
  installations?: GitHubAppInstallation[]
  repos?: { owner: string; name: string; default_branch: string | null; private: boolean }[]
}) {
  const { api } = await import("@/lib/api")
  vi.mocked(api.listWorkspaces).mockResolvedValue({
    workspaces: opts.workspaces ?? workspaces,
  })
  vi.mocked(api.listWorkspaceInstallations).mockResolvedValue({
    installations: opts.installations ?? installations,
  })
  vi.mocked(api.listWorkspaceProjects).mockResolvedValue({ projects: [] })
  vi.mocked(api.listInstallationRepos).mockResolvedValue({
    repos: opts.repos ?? [
      {
        owner: "onsager-ai",
        name: "onsager",
        default_branch: "main",
        private: false,
      },
    ],
  })
  vi.mocked(api.addWorkspaceProject).mockResolvedValue({
    project: {
      id: "proj_1",
      workspace_id: "ws_1",
      github_app_installation_id: "inst_one",
      repo_owner: "onsager-ai",
      repo_name: "onsager",
      default_branch: "main",
      created_at: "2026-01-01T00:00:00Z",
    },
  })
  vi.mocked(api.createWorkflow).mockResolvedValue({
    workflow: {
      id: "wf_1",
      workspace_id: "ws_1",
      name: "Auto-merge on green",
      preset: null,
      status: "active",
      trigger: {
        kind: "github-label",
        install_id: "1001",
        repo_owner: "onsager-ai",
        repo_name: "onsager",
        label: "auto-merge",
        kind_tag: "github_issue_webhook",
        manual_name: "",
      },
      stages: [],
      created_at: "2026-01-01T00:00:00Z",
      updated_at: "2026-01-01T00:00:00Z",
    },
  })
}

function mount(node: ReactNode) {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return render(
    <MemoryRouter>
      <QueryClientProvider client={qc}>{node}</QueryClientProvider>
    </MemoryRouter>,
  )
}

beforeEach(() => {
  vi.clearAllMocks()
  // Drafts persist to localStorage; reset per-test so `markDraftBound`
  // assertions don't bleed across cases.
  if (typeof window !== "undefined") {
    window.localStorage.clear()
  }
})

// ─── Tests ───────────────────────────────────────────────────────────────────

describe("BindDraftDialog (spec #402)", () => {
  it("skips Steps A and B when user has 1 workspace + 1 install", async () => {
    await primeApi({})
    const draft = makeBindableDraft()
    mount(
      <BindDraftDialog
        open
        onOpenChange={() => {}}
        draft={draft}
        userId="u_1"
      />,
    )
    // Step C copy: the repo picker heading.
    await waitFor(() =>
      expect(
        screen.getByText("Which repo runs this workflow?"),
      ).toBeInTheDocument(),
    )
    // The "Bind" button is visible.
    expect(screen.getByRole("button", { name: /^Bind/ })).toBeInTheDocument()
  })

  it("opens Step A when user has zero workspaces and shows the locked heading", async () => {
    await primeApi({ workspaces: [] })
    const draft = makeBindableDraft()
    mount(
      <BindDraftDialog
        open
        onOpenChange={() => {}}
        draft={draft}
        userId="u_1"
      />,
    )
    expect(
      await screen.findByText("First, name your factory floor."),
    ).toBeInTheDocument()
    expect(
      await screen.findByPlaceholderText("Personal"),
    ).toBeInTheDocument()
    expect(
      screen.getByRole("button", { name: /Create and continue/ }),
    ).toBeInTheDocument()
  })

  it("opens Step B when the chosen workspace has no installs", async () => {
    await primeApi({ installations: [] })
    const draft = makeBindableDraft()
    mount(
      <BindDraftDialog
        open
        onOpenChange={() => {}}
        draft={draft}
        userId="u_1"
      />,
    )
    await waitFor(() =>
      expect(
        screen.getByText("Give Onsager access to your repos."),
      ).toBeInTheDocument(),
    )
    const link = screen.getByRole("link", { name: /Install GitHub App/ })
    expect(link).toBeInTheDocument()
    expect(link.getAttribute("href")).toContain(
      "/api/github-app/install-start",
    )
    // Round-trip target — back to /chat with bind=continue.
    expect(link.getAttribute("href")).toContain("return_to=")
    expect(
      decodeURIComponent(link.getAttribute("href") ?? ""),
    ).toContain("bind=continue")
  })

  it("creates a workflow and marks the draft bound on Step C submit", async () => {
    await primeApi({})
    const draft = makeBindableDraft()
    const onOpenChange = vi.fn()
    mount(
      <BindDraftDialog
        open
        onOpenChange={onOpenChange}
        draft={draft}
        userId="u_1"
      />,
    )
    // Pick the repo via the combobox.
    await waitFor(() =>
      expect(
        screen.getByText("Which repo runs this workflow?"),
      ).toBeInTheDocument(),
    )
    fireEvent.click(screen.getByRole("combobox"))
    const item = await screen.findByText("onsager-ai/onsager")
    fireEvent.click(item)
    // Submit the bind.
    fireEvent.click(screen.getByRole("button", { name: /^Bind/ }))

    const { api } = await import("@/lib/api")
    await waitFor(() => expect(api.createWorkflow).toHaveBeenCalledTimes(1))
    expect(api.addWorkspaceProject).toHaveBeenCalledTimes(1)
    expect(onOpenChange).toHaveBeenCalledWith(false)

    // Draft persistence: `bound_to` is written under the user's slot.
    const raw = window.localStorage.getItem("onsager.drafts.u_1")
    expect(raw).not.toBeNull()
    const parsed = JSON.parse(raw!) as {
      drafts: { id: string; bound_to?: { workflow_id: string } }[]
    }
    const bound = parsed.drafts.find((d) => d.id === draft.id)
    expect(bound?.bound_to?.workflow_id).toBe("wf_1")
  })
})
