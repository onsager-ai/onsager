import { describe, it, expect } from "vitest"
import {
  createWorkflowRequestToBackend,
  type CreateWorkflowRequest,
} from "@/lib/api"

// The stiglab `validate_create_body` is the source of truth for the wire
// shape. These tests pin the UI → backend adapter against its contract:
// flat trigger fields, numeric install_id, snake_case `trigger_kind` /
// `trigger_label` / `active`, and stage params that carry the UI-only
// name + artifact_kind alongside any gate config.
describe("createWorkflowRequestToBackend", () => {
  const baseReq = (): CreateWorkflowRequest => ({
    tenant_id: "t_1",
    name: "Issue → PR",
    trigger: {
      kind: "github-label",
      install_id: "42",
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
    activate: true,
  })

  it("flattens the trigger and emits the backend enum values", () => {
    const out = createWorkflowRequestToBackend(baseReq())
    expect(out).toMatchObject({
      tenant_id: "t_1",
      name: "Issue → PR",
      trigger_kind: "github-issue-webhook",
      repo_owner: "onsager-ai",
      repo_name: "onsager",
      trigger_label: "factory",
      install_id: 42,
      active: true,
    })
    // No nested `trigger`, no `activate`, no `preset` — all wire keys.
    expect("trigger" in out).toBe(false)
    expect("activate" in out).toBe(false)
  })

  it("parses install_id to a number", () => {
    const out = createWorkflowRequestToBackend(baseReq())
    expect(typeof out.install_id).toBe("number")
    expect(out.install_id).toBe(42)
  })

  it("maps stages to { gate_kind, params } and carries UI display fields in params", () => {
    const out = createWorkflowRequestToBackend(baseReq())
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

  it("sends preset_id and drops stages when a preset is chosen", () => {
    const req = baseReq()
    req.preset = "github-issue-to-pr"
    const out = createWorkflowRequestToBackend(req)
    expect(out.preset_id).toBe("github-issue-to-pr")
    expect(out.stages).toBeUndefined()
  })

  it("defaults active=false when activate is not set", () => {
    const req = baseReq()
    delete req.activate
    expect(createWorkflowRequestToBackend(req).active).toBe(false)
  })

  it("throws when install_id can't be parsed", () => {
    const req = baseReq()
    req.trigger.install_id = ""
    expect(() => createWorkflowRequestToBackend(req)).toThrow(/install_id/)
  })
})
