import { describe, it, expect, vi, beforeEach } from "vitest"
import { render, screen } from "@testing-library/react"
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"

import { ArtifactContent } from "@/components/artifacts/ArtifactContent"
import type { ArtifactDetail } from "@/lib/api/generated/ArtifactDetail"

// ADR 0021 / #491 + #492: the `github_issue` Content branch absorbs the
// rendering that used to live in the retired IssueDetailPage — live
// title / body / labels / assignees plus the proxy fail-open banner.

const apiMock = vi.hoisted(() => ({
  getProjectIssue: vi.fn(),
  replayIssueTrigger: vi.fn(),
}))

vi.mock("@/lib/api", async () => {
  const actual =
    await vi.importActual<typeof import("@/lib/api")>("@/lib/api")
  return { ...actual, api: apiMock }
})

function baseArtifact(overrides: Partial<ArtifactDetail>): ArtifactDetail {
  return {
    id: "art_1",
    kind: "github_issue",
    name: null,
    state: "draft",
    owner: null,
    current_version: 1,
    consumers: [],
    external_ref: "github:project:proj_1:issue:42",
    created_at: "2026-04-01T00:00:00Z",
    updated_at: "2026-04-29T00:00:00Z",
    last_observed_at: "2026-04-29T00:00:00Z",
    created_by: "engine",
    versions: [],
    vertical_lineage: [],
    horizontal_lineage: [],
    related_events: [],
    ...overrides,
  }
}

function renderContent(artifact: ArtifactDetail) {
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  return render(
    <QueryClientProvider client={qc}>
      <ArtifactContent artifact={artifact} />
    </QueryClientProvider>,
  )
}

describe("ArtifactContent github_issue branch", () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it("renders the live title, body, labels, and assignees", async () => {
    apiMock.getProjectIssue.mockResolvedValue({
      issue: {
        number: 42,
        title: "Add issue detail page",
        state: "open",
        html_url: "https://github.com/acme/widgets/issues/42",
        author: "alice",
        labels: ["spec", "area:dashboard"],
        assignees: ["bob"],
        comments: 3,
        body: "We should have an issue detail page.",
        milestone: null,
        created_at: "2026-04-01T00:00:00Z",
        updated_at: "2026-04-29T00:00:00Z",
        closed_at: null,
      },
    })

    renderContent(baseArtifact({}))

    expect(
      await screen.findByText("Add issue detail page"),
    ).toBeInTheDocument()
    expect(
      screen.getByText("We should have an issue detail page."),
    ).toBeInTheDocument()
    expect(screen.getByText("spec")).toBeInTheDocument()
    expect(screen.getByText("area:dashboard")).toBeInTheDocument()
    expect(screen.getByText("bob")).toBeInTheDocument()
    // Parsed the projectId + number out of the external_ref.
    expect(apiMock.getProjectIssue).toHaveBeenCalledWith("proj_1", 42)
  })

  it("surfaces the rate-limit banner when the proxy fails open", async () => {
    apiMock.getProjectIssue.mockResolvedValue({
      issue: null,
      error: "rate_limited",
    })

    renderContent(baseArtifact({}))

    expect(
      await screen.findByText(/GitHub rate limit reached/),
    ).toBeInTheDocument()
  })

  it("surfaces a degraded banner when issue hydration throws (404/network)", async () => {
    apiMock.getProjectIssue.mockRejectedValue(new Error("not found"))

    renderContent(baseArtifact({}))

    expect(
      await screen.findByText(/Couldn't load the live issue from GitHub/),
    ).toBeInTheDocument()
  })

  it("non-issue kinds fall through to a content branch, not the issue proxy", async () => {
    renderContent(
      baseArtifact({
        kind: "markdown",
        external_ref: null,
        versions: [
          {
            version: 1,
            content_ref_uri: "s3://bucket/doc.md",
            content_ref_checksum: null,
            change_summary: "Initial draft",
            created_by_session: "sess_1",
            parent_version: null,
            created_at: "2026-04-01T00:00:00Z",
          },
        ],
      }),
    )

    expect(await screen.findByText("Initial draft")).toBeInTheDocument()
    expect(apiMock.getProjectIssue).not.toHaveBeenCalled()
  })
})
