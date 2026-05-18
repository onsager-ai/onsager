import { describe, it, expect, vi, beforeEach, afterEach } from "vitest"
import { render, screen, waitFor } from "@testing-library/react"
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { MemoryRouter, Routes, Route } from "react-router-dom"

import { WorkflowDetailPage } from "@/pages/WorkflowDetailPage"
import { DraftStrip } from "@/components/chat/DraftStrip"

// Spec #405: Cloud-vs-OSS capability boundary surfacing. Three inline
// lines surface only when `is_oss === true`, exactly where the user is
// hitting the natural limit. This test pins each of the three locations
// behaves correctly with and without the OSS flag.

vi.mock("@/lib/auth", () => ({
  useAuth: () => ({ user: null, authEnabled: false }),
}))

const apiMock = vi.hoisted(() => ({
  listWorkspaces: vi.fn(),
  getWorkflow: vi.fn(),
  getWorkflowRuns: vi.fn(),
  listWorkflows: vi.fn(),
}))

vi.mock("@/lib/api", async () => {
  const actual =
    await vi.importActual<typeof import("@/lib/api")>("@/lib/api")
  return { ...actual, api: apiMock }
})

const buildInfoMock = vi.hoisted(() => ({ useOSSFlag: vi.fn() }))
vi.mock("@/hooks/useOSSFlag", () => buildInfoMock)

import { WorkspaceScope } from "@/lib/workspace"

const workspace = {
  id: "ws_1",
  slug: "acme",
  name: "Acme",
  created_by: "u1",
  created_at: "2026-01-01",
}

function makeWorkflow(kindTag: string) {
  return {
    workflow: {
      id: "wf_1",
      workspace_id: "ws_1",
      name: "Issue → PR",
      preset: null,
      status: "active",
      trigger: {
        kind: "github-label",
        install_id: "42",
        repo_owner: "onsager-ai",
        repo_name: "onsager",
        label: "factory",
        kind_tag: kindTag,
      },
      stages: [
        {
          id: "s_1",
          name: "Triage",
          gate_kind: "agent-session",
          artifact_kind: "Issue",
          config: {},
        },
      ],
      created_at: "2026-04-22T00:00:00Z",
      updated_at: "2026-04-22T00:00:00Z",
    },
  }
}

function renderWorkflow(hash: string) {
  // Tab state seeds from `window.location.hash` (not from React Router),
  // so we mirror the existing detail-page test pattern and pin the
  // browser-side URL before render. MemoryRouter alone does not set
  // window.location.
  window.history.replaceState(
    null,
    "",
    `/workspaces/acme/workflows/wf_1${hash}`,
  )
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  return render(
    <QueryClientProvider client={qc}>
      <MemoryRouter initialEntries={[`/workspaces/acme/workflows/wf_1${hash}`]}>
        <Routes>
          <Route
            path="/workspaces/:workspace/*"
            element={
              <WorkspaceScope>
                <Routes>
                  <Route
                    path="workflows/:id"
                    element={<WorkflowDetailPage />}
                  />
                </Routes>
              </WorkspaceScope>
            }
          />
        </Routes>
      </MemoryRouter>
    </QueryClientProvider>,
  )
}

describe("Spec #405 — OSS capability-boundary surfacing", () => {
  beforeEach(() => {
    vi.clearAllMocks()
    window.history.replaceState(null, "", "/workspaces/acme/workflows/wf_1")
    apiMock.listWorkspaces.mockResolvedValue({ workspaces: [workspace] })
    apiMock.getWorkflowRuns.mockResolvedValue({ runs: [] })
    apiMock.listWorkflows.mockResolvedValue({ workflows: [] })
  })
  afterEach(() => {
    window.location.hash = ""
  })

  it("renders the 7-day cap line on the Runs tab when OSS", async () => {
    buildInfoMock.useOSSFlag.mockReturnValue(true)
    apiMock.getWorkflow.mockResolvedValue(makeWorkflow("github_issue_webhook"))
    renderWorkflow("#runs")
    await waitFor(() =>
      expect(
        screen.getByText("Showing last 7 days · Cloud retains 90."),
      ).toBeTruthy(),
    )
  })

  it("hides the 7-day cap line on the Runs tab when Cloud", async () => {
    buildInfoMock.useOSSFlag.mockReturnValue(false)
    apiMock.getWorkflow.mockResolvedValue(makeWorkflow("github_issue_webhook"))
    renderWorkflow("#runs")
    // Wait for the Run history card to render before asserting absence.
    await screen.findByText("Run history")
    expect(
      screen.queryByText("Showing last 7 days · Cloud retains 90."),
    ).toBeNull()
  })

  it("renders the scheduler limitation line on schedule triggers when OSS", async () => {
    buildInfoMock.useOSSFlag.mockReturnValue(true)
    apiMock.getWorkflow.mockResolvedValue(makeWorkflow("cron"))
    renderWorkflow("#definition")
    await waitFor(() =>
      expect(
        screen.getByText(
          /Runs while this Onsager process is running\. For 24\/7 schedules/,
        ),
      ).toBeTruthy(),
    )
    const link = screen.getByRole("link", { name: /use Cloud/ })
    expect(link.getAttribute("href")).toBe("https://app.onsager.ai")
  })

  it("hides the scheduler limitation line on non-schedule triggers", async () => {
    buildInfoMock.useOSSFlag.mockReturnValue(true)
    apiMock.getWorkflow.mockResolvedValue(makeWorkflow("github_issue_webhook"))
    renderWorkflow("#definition")
    // Definition tab renders the Flow / stages; wait for the canonical
    // stage marker, then assert the schedule copy is absent.
    await screen.findByText("Stages")
    expect(
      screen.queryByText(/Runs while this Onsager process is running/),
    ).toBeNull()
  })

  it("hides the scheduler limitation line on schedule triggers when Cloud", async () => {
    buildInfoMock.useOSSFlag.mockReturnValue(false)
    apiMock.getWorkflow.mockResolvedValue(makeWorkflow("cron"))
    renderWorkflow("#definition")
    await screen.findByText("Stages")
    expect(
      screen.queryByText(/Runs while this Onsager process is running/),
    ).toBeNull()
  })
})

describe("Spec #405 — DraftStrip OSS footer", () => {
  const draft = {
    id: "d_1",
    name: "Untitled draft",
    updated_at: new Date().toISOString(),
    workflow: { name: "", stages: [], trigger: null },
    template_id: undefined,
    // The strip only reads `id`, `name`, `updated_at`; cast keeps the
    // test fixture small without dragging in the full draft shape.
  } as unknown as Parameters<typeof DraftStrip>[0]["drafts"][number]

  it("renders the `Drafts on this device.` footer when OSS", () => {
    render(
      <DraftStrip
        drafts={[draft]}
        activeId={draft.id}
        onSwitch={() => {}}
        onNew={() => {}}
        onDelete={() => {}}
        isOss={true}
      />,
    )
    expect(screen.getByText("Drafts on this device.")).toBeTruthy()
  })

  it("hides the footer when Cloud", () => {
    render(
      <DraftStrip
        drafts={[draft]}
        activeId={draft.id}
        onSwitch={() => {}}
        onNew={() => {}}
        onDelete={() => {}}
        isOss={false}
      />,
    )
    expect(screen.queryByText("Drafts on this device.")).toBeNull()
  })

  it("hides the footer when there are no drafts (strip is hidden entirely)", () => {
    const { container } = render(
      <DraftStrip
        drafts={[]}
        activeId={null}
        onSwitch={() => {}}
        onNew={() => {}}
        onDelete={() => {}}
        isOss={true}
      />,
    )
    expect(container.firstChild).toBeNull()
  })
})
