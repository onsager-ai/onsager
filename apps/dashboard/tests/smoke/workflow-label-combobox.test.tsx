import { describe, it, expect, vi } from "vitest"
import { render, screen, fireEvent, waitFor } from "@testing-library/react"
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { LabelCombobox } from "@/components/factory/workflows/LabelCombobox"
import type { GitHubLabel } from "@/lib/api"

const labels: GitHubLabel[] = [
  { name: "bug", color: "d73a4a", description: null },
  { name: "feat", color: "0e8a16", description: null },
  { name: "chore", color: "cccccc", description: null },
]

function mount(node: React.ReactNode) {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return render(<QueryClientProvider client={qc}>{node}</QueryClientProvider>)
}

describe("LabelCombobox", () => {
  it("lists existing labels", async () => {
    mount(
      <LabelCombobox
        tenantId="t1"
        installId="i1"
        repoOwner="onsager-ai"
        repoName="onsager"
        value={null}
        onChange={vi.fn()}
        labelsOverride={labels}
        disableFetch
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
        tenantId="t1"
        installId="i1"
        repoOwner="onsager-ai"
        repoName="onsager"
        value={null}
        onChange={vi.fn()}
        labelsOverride={labels}
        disableFetch
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
        tenantId="t1"
        installId="i1"
        repoOwner="onsager-ai"
        repoName="onsager"
        value={null}
        onChange={onChange}
        labelsOverride={labels}
        disableFetch
      />,
    )
    fireEvent.click(screen.getByRole("combobox"))
    const item = await screen.findByText("bug")
    fireEvent.click(item)
    expect(onChange).toHaveBeenCalledWith("bug")
  })
})
