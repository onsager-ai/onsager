import { describe, it, expect } from "vitest"
import { deriveAutoScope } from "@/lib/chat/use-auto-scope"

// Route → scope mapping (ADR 0020 § Contextual auto-scope, spec #471
// Plan: "Hook test: route → scope mapping for each of the four scope
// types"). The hook itself is route-reactive — the pure derivation is
// unit-tested here; the React-level pin/unpin behavior is covered by
// the smoke tests.

describe("deriveAutoScope", () => {
  it("maps the workflows list to workspace scope", () => {
    expect(deriveAutoScope("/workspaces/acme/workflows", "")).toEqual({
      type: "workspace",
      id: null,
    })
  })

  it("maps the workflow detail to workflow:{id}", () => {
    expect(deriveAutoScope("/workspaces/acme/workflows/wf-1", "")).toEqual({
      type: "workflow",
      id: "wf-1",
    })
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
