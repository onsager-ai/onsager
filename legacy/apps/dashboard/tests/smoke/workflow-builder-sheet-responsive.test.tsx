import { describe, it, expect, vi, beforeEach } from "vitest"
import { render, waitFor } from "@testing-library/react"
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { MemoryRouter } from "react-router-dom"
import type { ReactNode } from "react"

import { WorkflowBuilderSheet } from "@/components/workflows/WorkflowBuilderSheet"

// The builder mounts inside a centered Dialog on desktop and a bottom Sheet
// on mobile (spec #560). Both wrap the same base-ui dialog primitive, so the
// branch is observable only via the distinct content `data-slot`s. Mock the
// breakpoint hook so the test pins the branch rather than the viewport.
vi.mock("@/hooks/use-mobile", () => ({
  useIsMobile: vi.fn(),
}))

vi.mock("@/lib/api", async () => {
  const actual = await vi.importActual<{ api: Record<string, unknown> }>(
    "@/lib/api",
  )
  return {
    ...actual,
    api: {
      ...actual.api,
      listWorkspaces: vi.fn().mockResolvedValue({ workspaces: [] }),
      listWorkspaceInstallations: vi
        .fn()
        .mockResolvedValue({ installations: [] }),
    },
  }
})

async function setMobile(value: boolean) {
  const { useIsMobile } = await import("@/hooks/use-mobile")
  vi.mocked(useIsMobile).mockReturnValue(value)
}

function mount(node: ReactNode) {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return render(
    <QueryClientProvider client={qc}>
      <MemoryRouter>{node}</MemoryRouter>
    </QueryClientProvider>,
  )
}

describe("WorkflowBuilderSheet responsive container", () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it("renders a centered Dialog (not a Sheet) on desktop", async () => {
    await setMobile(false)
    mount(<WorkflowBuilderSheet open onOpenChange={() => {}} />)
    await waitFor(() =>
      expect(
        document.querySelector('[data-slot="dialog-content"]'),
      ).not.toBeNull(),
    )
    expect(document.querySelector('[data-slot="sheet-content"]')).toBeNull()
  })

  it("renders a bottom Sheet (not a Dialog) on mobile", async () => {
    await setMobile(true)
    mount(<WorkflowBuilderSheet open onOpenChange={() => {}} />)
    await waitFor(() =>
      expect(
        document.querySelector('[data-slot="sheet-content"]'),
      ).not.toBeNull(),
    )
    expect(document.querySelector('[data-slot="dialog-content"]')).toBeNull()
  })
})
