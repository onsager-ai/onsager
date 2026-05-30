import { describe, it, expect } from "vitest"
import { deriveAutoScope } from "@/lib/chat/use-auto-scope"

// Route → default chat scope (ADR 0020 § Contextual auto-scope, narrowed
// by ADR 0025 § Chat's scope narrows to two jobs, spec #514). The default
// scope a bare route resolves to is design (workspace) or maintenance
// (run / verdict) — never a workflow scope. The `workflow` scope survives
// only as an explicit ChatAskButton override, exercised in the smoke
// tests, not as a route default.

describe("deriveAutoScope (design + maintenance only)", () => {
  it("maps the workflows list to workspace scope", () => {
    expect(deriveAutoScope("/workspaces/acme/workflows", "")).toEqual({
      type: "workspace",
      id: null,
    })
  })

  it("maps workflow detail to workspace (design), not workflow scope", () => {
    // ADR 0025: a workflow is *designed* at the workspace level; its
    // detail route no longer auto-scopes to workflow:{id}.
    expect(deriveAutoScope("/workspaces/acme/workflows/wf-1", "")).toEqual({
      type: "workspace",
      id: null,
    })
  })

  it("never returns a workflow scope from any bare route", () => {
    for (const path of [
      "/workspaces/acme/workflows/wf-1",
      "/workspaces/acme/workflows/start",
      "/workspaces/acme/spec-plans",
    ]) {
      expect(deriveAutoScope(path, "").type).not.toBe("workflow")
    }
  })

  it("does not treat /workflows/start as a workflow scope", () => {
    // /workflows/start is the creation surface — workspace-scoped.
    expect(deriveAutoScope("/workspaces/acme/workflows/start", "")).toEqual({
      type: "workspace",
      id: null,
    })
  })

  it("maps the runs list to workspace scope", () => {
    expect(deriveAutoScope("/workspaces/acme/runs", "")).toEqual({
      type: "workspace",
      id: null,
    })
  })

  it("maps the run detail to run:{id}", () => {
    expect(deriveAutoScope("/workspaces/acme/runs/run-42", "")).toEqual({
      type: "run",
      id: "run-42",
    })
  })

  it("uses the ?verdict= deep link for verdict scope", () => {
    expect(
      deriveAutoScope("/workspaces/acme/runs/run-42", "?verdict=vd-9"),
    ).toEqual({ type: "verdict", id: "vd-9" })
  })

  it("maps activity to workspace scope (tab switch preserves level)", () => {
    expect(deriveAutoScope("/workspaces/acme/activity", "")).toEqual({
      type: "workspace",
      id: null,
    })
  })

  it("maps settings to workspace scope", () => {
    expect(deriveAutoScope("/workspaces/acme/settings", "")).toEqual({
      type: "workspace",
      id: null,
    })
  })

  it("falls back to workspace scope outside /workspaces/*", () => {
    expect(deriveAutoScope("/settings", "")).toEqual({
      type: "workspace",
      id: null,
    })
    expect(deriveAutoScope("/workspaces", "")).toEqual({
      type: "workspace",
      id: null,
    })
  })

  it("ignores trailing slashes in run detail", () => {
    expect(deriveAutoScope("/workspaces/acme/runs/run-42/", "")).toEqual({
      type: "workspace",
      id: null,
    })
  })
})
