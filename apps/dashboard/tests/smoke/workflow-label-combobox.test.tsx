import { describe, it, expect, vi } from "vitest"
import { render, screen, fireEvent, waitFor } from "@testing-library/react"
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import type { ReactNode } from "react"
import { LabelCombobox } from "@/components/factory/workflows/LabelCombobox"
import type { GitHubLabel } from "@/lib/api"

const labels: GitHubLabel[] = [
  { name: "bug", color: "d73a4a", description: null },
  { name: "feat", color: "0e8a16", description: null },
  { name: "chore", color: "cccccc", description: null },
]

function mount(node: ReactNode, labelsSeed: GitHubLabel[] = labels) {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  // Pre-seed the label query so the component reads cached data without
  // issuing a network request.
  qc.setQueryData(["repo-labels", "t1", "i1", "onsager-ai", "onsager"], {
    labels: labelsSeed,
  })
  return render(<QueryClientProvider client={qc}>{node}</QueryClientProvider>)
}

describe("LabelCombobox", () => {
  it("lists existing labels", async () => {
    mount(
      <LabelCombobox
        workspaceId="t1"
        installId="i1"
        repoOwner="onsager-ai"
        repoName="onsager"
        value={null}
        onChange={vi.fn()}
      />,
    )
    fireEvent.click(screen.getByRole("combobox"))
    await waitFor(() => expect(screen.getByText("bug")).toBeInTheDocument())
    expect(screen.getByText("feat")).toBeInTheDocument()
    expect(screen.getByText("chore")).toBeInTheDocument()
  })

  it("surfaces an inline-create affordance when the query has no exact match", async () => {
    mount(
      <LabelCombobox
        workspaceId="t1"
        installId="i1"
        repoOwner="onsager-ai"
        repoName="onsager"
        value={null}
        onChange={vi.fn()}
      />,
    )
    fireEvent.click(screen.getByRole("combobox"))
    const input = await screen.findByPlaceholderText("Search or create a label…")
    fireEvent.change(input, { target: { value: "release" } })
    await waitFor(() =>
      expect(screen.getByText(/Create label "release"/)).toBeInTheDocument(),
    )
  })

  it("commits a structured label on select (not free text)", async () => {
    const onChange = vi.fn()
    mount(
      <LabelCombobox
        workspaceId="t1"
        installId="i1"
        repoOwner="onsager-ai"
        repoName="onsager"
        value={null}
        onChange={onChange}
      />,
    )
    fireEvent.click(screen.getByRole("combobox"))
    const item = await screen.findByText("bug")
    fireEvent.click(item)
    expect(onChange).toHaveBeenCalledWith("bug")
  })
})
