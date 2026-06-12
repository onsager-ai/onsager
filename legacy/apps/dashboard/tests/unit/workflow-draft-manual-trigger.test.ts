import { describe, it, expect } from "vitest"
import type { GitHubAppInstallation } from "@/lib/api"
import {
  documentToCreateRequest,
  emptyDocument,
  isTriggerReady,
  makeStage,
  type WorkflowDocument,
  type WorkflowTriggerDraft,
} from "@/components/workflows/workflow-draft"

// Manual repo-less trigger kind in the workflow builder (spec #561).
// The backend is unchanged — these exercise the dashboard's draft-side
// branch on `kind_tag`: Manual carries no config of its own (no install,
// repo, label, or name — the fire button derives its label from the
// workflow name), so it's always ready. GitHub keeps requiring install +
// repo + label (#545 — no regression).

function githubTrigger(
  over: Partial<WorkflowTriggerDraft> = {},
): WorkflowTriggerDraft {
  return {
    kind_tag: "github_issue_webhook",
    install_id: "inst_1",
    repo_owner: "acme",
    repo_name: "widgets",
    label: "ai",
    ...over,
  }
}

function manualTrigger(
  over: Partial<WorkflowTriggerDraft> = {},
): WorkflowTriggerDraft {
  return {
    kind_tag: "manual",
    install_id: "",
    repo_owner: "",
    repo_name: "",
    label: "",
    ...over,
  }
}

const INSTALLATIONS: GitHubAppInstallation[] = [
  {
    id: "inst_1",
    workspace_id: "ws_1",
    install_id: 4242,
    account_login: "acme",
    account_type: "Organization",
    created_at: "2026-01-01T00:00:00Z",
  },
]

describe("isTriggerReady — per-kind branch (#561)", () => {
  it("Manual is always ready — it carries no config of its own", () => {
    expect(isTriggerReady(manualTrigger())).toBe(true)
  })

  it("Manual ignores install/repo/label entirely", () => {
    // No install, no repo, no label — still ready; a manual trigger has no
    // fields to fill (the fire button uses the workflow name).
    expect(
      isTriggerReady(
        manualTrigger({
          install_id: "",
          repo_owner: "",
          repo_name: "",
          label: "",
        }),
      ),
    ).toBe(true)
  })

  it("GitHub still requires install + repo + label (no regression)", () => {
    expect(isTriggerReady(githubTrigger())).toBe(true)
    expect(isTriggerReady(githubTrigger({ install_id: "" }))).toBe(false)
    expect(isTriggerReady(githubTrigger({ repo_owner: "" }))).toBe(false)
    expect(isTriggerReady(githubTrigger({ repo_name: "" }))).toBe(false)
    expect(isTriggerReady(githubTrigger({ label: "" }))).toBe(false)
  })

  it("a GitHub trigger missing repo/label is NOT ready (kind decides)", () => {
    // Mutation guard: if the Manual branch keyed off something other than
    // `kind_tag`, this github trigger (missing repo/label) would falsely
    // pass. It must stay false because the kind is github_issue_webhook.
    expect(
      isTriggerReady(
        githubTrigger({
          install_id: "",
          repo_owner: "",
          repo_name: "",
          label: "",
        }),
      ),
    ).toBe(false)
  })
})

describe("documentToCreateRequest — Manual variant (#561)", () => {
  function manualDoc(): WorkflowDocument {
    return {
      ...emptyDocument(),
      name: "Nightly batch",
      trigger: manualTrigger(),
      stages: [makeStage("agent-session", "Issue", "Agent session")],
    }
  }

  it("emits a repo-less manual trigger named after the workflow", () => {
    const body = documentToCreateRequest(manualDoc(), [], "ws_1", false)
    // The manual fire label is derived from the workflow name — there is no
    // separate trigger name to author (#581).
    expect(body.trigger).toEqual({ kind: "manual", name: "Nightly batch" })
    // Repo-less: no install token-mint hint, no repo on the trigger.
    expect(body.install_id).toBeUndefined()
    expect("repo" in body.trigger).toBe(false)
    expect(body.workspace_id).toBe("ws_1")
    expect(body.active).toBe(false)
  })

  it("succeeds for a Manual workflow with NO repo selected", () => {
    // The "repo unpinned" leg (deferred from #547 / #553): a Manual
    // workflow created with no install/repo must not throw.
    expect(() =>
      documentToCreateRequest(manualDoc(), [], "ws_1", true),
    ).not.toThrow()
  })

  it("trims the workflow name when deriving the manual fire label", () => {
    const doc = manualDoc()
    doc.name = "  spaced  "
    const body = documentToCreateRequest(doc, [], "ws_1", false)
    expect(body.trigger).toEqual({ kind: "manual", name: "spaced" })
  })
})

describe("documentToCreateRequest — GitHub variant unchanged (#561)", () => {
  function githubDoc(): WorkflowDocument {
    return {
      ...emptyDocument(),
      name: "Issue to PR",
      trigger: githubTrigger(),
      stages: [makeStage("agent-session", "Issue", "Agent session")],
    }
  }

  it("emits the github_issue_webhook variant with repo + numeric install id", () => {
    const body = documentToCreateRequest(githubDoc(), INSTALLATIONS, "ws_1", true)
    expect(body.trigger).toEqual({
      kind: "github_issue_webhook",
      repo: "acme/widgets",
      label: "ai",
    })
    expect(body.install_id).toBe(4242)
    expect(body.active).toBe(true)
  })

  it("still requires the selected install to exist in the workspace", () => {
    expect(() =>
      documentToCreateRequest(githubDoc(), [], "ws_1", false),
    ).toThrow()
  })
})
