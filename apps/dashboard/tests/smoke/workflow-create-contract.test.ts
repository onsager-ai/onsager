import { describe, it, expect } from "vitest"
import type { GitHubAppInstallation } from "@/lib/api"
import {
  draftToCreateRequest,
  emptyDraft,
  type WorkflowDraft,
} from "@/components/factory/workflows/workflow-draft"

// The stiglab `validate_create_body` is the source of truth for the wire
// shape. These tests pin the UI → backend adapter against its contract:
// flat trigger fields, numeric install_id resolved from the install record,
// snake_case `trigger_kind` / `trigger_label` / `active`, and stage params
// that carry the UI-only name + artifact_kind alongside any gate config.
describe("draftToCreateRequest", () => {
  const installation: GitHubAppInstallation = {
    id: "inst_abc",
    workspace_id: "t_1",
    install_id: 12345,
    account_login: "onsager-ai",
    account_type: "organization",
    created_at: "2026-04-22T00:00:00Z",
  }

  const draft = (): WorkflowDraft => ({
    name: "Issue → PR",
    trigger: {
      install_id: "inst_abc", // record id, not numeric GitHub install id
      repo_owner: "onsager-ai",
      repo_name: "onsager",
      label: "factory",
    },
    stages: [
      {
        id: "s1",
        name: "Spec → PR",
        gate_kind: "agent-session",
        artifact_kind: "github-issue",
        config: { agent_profile: "default" },
      },
    ],
  })

  it("resolves the numeric GitHub install_id from the installations list", () => {
    const out = draftToCreateRequest(draft(), [installation], "t_1", true)
    expect(out.install_id).toBe(12345)
    expect(typeof out.install_id).toBe("number")
  })

  it("flattens the trigger into the backend's snake_case keys", () => {
    const out = draftToCreateRequest(draft(), [installation], "t_1", true)
    expect(out).toMatchObject({
      workspace_id: "t_1",
      name: "Issue → PR",
      trigger_kind: "github_issue_webhook",
      repo_owner: "onsager-ai",
      repo_name: "onsager",
      trigger_label: "factory",
      active: true,
    })
    // No nested `trigger`, no `activate`, no `preset` — all wire keys.
    expect("trigger" in out).toBe(false)
    expect("activate" in out).toBe(false)
  })

  it("maps stages to { gate_kind, params } and carries UI display fields in params", () => {
    const out = draftToCreateRequest(draft(), [installation], "t_1", true)
    expect(out.stages).toEqual([
      {
        gate_kind: "agent-session",
        params: {
          agent_profile: "default",
          name: "Spec → PR",
          artifact_kind: "github-issue",
        },
      },
    ])
  })

  it("passes activate=false through as active=false", () => {
    const out = draftToCreateRequest(draft(), [installation], "t_1", false)
    expect(out.active).toBe(false)
  })

  it("throws when workspace_id is blank", () => {
    expect(() => draftToCreateRequest(draft(), [installation], "  ", true)).toThrow(
      /workspace_id/,
    )
  })

  it("throws when the selected install isn't in the list", () => {
    expect(() => draftToCreateRequest(draft(), [], "t_1", true)).toThrow(
      /install not found/,
    )
  })

  it("throws when the trigger isn't ready (missing label)", () => {
    const d = emptyDraft()
    expect(() => draftToCreateRequest(d, [installation], "t_1", true)).toThrow(
      /install, repo, and label/,
    )
  })
})
