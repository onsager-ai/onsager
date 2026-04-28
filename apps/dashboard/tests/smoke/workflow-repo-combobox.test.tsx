import { describe, it, expect, vi } from "vitest"
import { render, screen, fireEvent, waitFor } from "@testing-library/react"
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import type { ReactNode } from "react"
import { RepoCombobox } from "@/components/factory/workflows/RepoCombobox"
import type {
  AccessibleRepo,
  GitHubAppInstallation,
} from "@/lib/api"

const installations: GitHubAppInstallation[] = [
  {
    id: "inst_one",
    workspace_id: "t1",
    install_id: 1001,
    account_login: "onsager-ai",
    account_type: "organization",
    created_at: "2026-01-01T00:00:00Z",
  },
  {
    id: "inst_two",
    workspace_id: "t1",
    install_id: 1002,
    account_login: "tikazyq",
    account_type: "user",
    created_at: "2026-01-01T00:00:00Z",
  },
]

const reposByInstall: Record<string, AccessibleRepo[]> = {
  inst_one: [
    {
      owner: "onsager-ai",
      name: "onsager",
      default_branch: "main",
      private: false,
    },
    {
      owner: "onsager-ai",
      name: "spec-bot",
      default_branch: "main",
      private: true,
    },
  ],
  inst_two: [
    {
      owner: "tikazyq",
      name: "playground",
      default_branch: "main",
      private: false,
    },
  ],
}

function mount(node: ReactNode) {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  // Pre-seed the per-install repo queries so opening the popover reads
  // cached data without firing real network calls.
  for (const inst of installations) {
    qc.setQueryData(
      ["installation-repos", "t1", inst.id],
      { repos: reposByInstall[inst.id] },
    )
  }
  return render(<QueryClientProvider client={qc}>{node}</QueryClientProvider>)
}

describe("RepoCombobox", () => {
  it("groups repos under each install's account login", async () => {
    mount(
      <RepoCombobox
        workspaceId="t1"
        installations={installations}
        installId=""
        repoOwner=""
        repoName=""
        onChange={vi.fn()}
      />,
    )
    fireEvent.click(screen.getByRole("combobox"))
    await waitFor(() =>
      expect(screen.getByText("onsager-ai/onsager")).toBeInTheDocument(),
    )
    expect(screen.getByText("onsager-ai/spec-bot")).toBeInTheDocument()
    expect(screen.getByText("tikazyq/playground")).toBeInTheDocument()
    expect(
      screen.getByText(/onsager-ai \(organization\)/),
    ).toBeInTheDocument()
    expect(screen.getByText(/tikazyq \(user\)/)).toBeInTheDocument()
  })

  it("emits install_id, repo_owner, and repo_name in one shot on select", async () => {
    const onChange = vi.fn()
    mount(
      <RepoCombobox
        workspaceId="t1"
        installations={installations}
        installId=""
        repoOwner=""
        repoName=""
        onChange={onChange}
      />,
    )
    fireEvent.click(screen.getByRole("combobox"))
    const item = await screen.findByText("onsager-ai/spec-bot")
    fireEvent.click(item)
    expect(onChange).toHaveBeenCalledWith({
      install_id: "inst_one",
      repo_owner: "onsager-ai",
      repo_name: "spec-bot",
    })
  })

  it("offers a deep-link to configure repository access on GitHub", async () => {
    mount(
      <RepoCombobox
        workspaceId="t1"
        installations={installations}
        installId="inst_one"
        repoOwner=""
        repoName=""
        onChange={vi.fn()}
      />,
    )
    fireEvent.click(screen.getByRole("combobox"))
    await waitFor(() =>
      expect(
        screen.getByText("Configure repository access on GitHub"),
      ).toBeInTheDocument(),
    )
  })
})
