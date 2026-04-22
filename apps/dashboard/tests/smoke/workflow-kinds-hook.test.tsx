import { describe, it, expect, vi, beforeEach, afterEach } from "vitest"
import { render, screen, waitFor } from "@testing-library/react"
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import type { ReactNode } from "react"
import { ArtifactBadge } from "@/components/factory/workflows/ArtifactBadge"
import { api, type WorkflowKindInfo } from "@/lib/api"

function mount(node: ReactNode) {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return render(<QueryClientProvider client={qc}>{node}</QueryClientProvider>)
}

describe("useWorkflowKinds (issue #102 runtime fetch)", () => {
  beforeEach(() => {
    vi.spyOn(api, "listWorkflowKinds")
  })
  afterEach(() => {
    vi.restoreAllMocks()
  })

  it("resolves a registry alias to the canonical kind label", async () => {
    const kinds: WorkflowKindInfo[] = [
      {
        id: "Issue",
        description: "GitHub Issue",
        merge_rule: "overwrite",
        aliases: ["Spec", "github-issue"],
        intrinsic_schema: null,
      },
    ]
    vi.mocked(api.listWorkflowKinds).mockResolvedValue({ kinds })

    // `Spec` is a legacy id that the registry reports as an alias for
    // `Issue` — the badge should render the canonical short label once
    // the fetch resolves.
    mount(<ArtifactBadge kind="Spec" />)
    await waitFor(() => expect(screen.getByText("Issue")).toBeTruthy())
  })

  it("falls back to the static meta when the registry fetch fails", async () => {
    vi.mocked(api.listWorkflowKinds).mockRejectedValue(new Error("offline"))

    mount(<ArtifactBadge kind="PR" />)
    // Static fallback is synchronous, so the short label is visible on
    // the first render without waiting for the fetch to settle.
    expect(screen.getByText("PR")).toBeTruthy()
  })
})
