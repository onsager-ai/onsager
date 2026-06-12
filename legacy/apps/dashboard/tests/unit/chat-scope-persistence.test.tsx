import { describe, it, expect, beforeEach } from "vitest"
import {
  scopeKey,
  parseScopeKey,
  scopesEqual,
  WORKSPACE_SCOPE,
} from "@/lib/chat/scope"
import {
  readPinnedScope,
  writePinnedScope,
  clearPinnedScope,
} from "@/lib/chat/pinned-scope"

beforeEach(() => {
  window.localStorage.clear()
})

describe("scope key round-trip", () => {
  it("serializes and parses workspace scope", () => {
    expect(scopeKey(WORKSPACE_SCOPE)).toBe("workspace")
    expect(parseScopeKey("workspace")).toEqual(WORKSPACE_SCOPE)
  })

  it("serializes and parses workflow scope", () => {
    const s = { type: "workflow", id: "wf-1" } as const
    expect(scopeKey(s)).toBe("workflow:wf-1")
    expect(parseScopeKey("workflow:wf-1")).toEqual(s)
  })

  it("scopesEqual matches by both type and id", () => {
    expect(
      scopesEqual(
        { type: "run", id: "r-1" },
        { type: "run", id: "r-1" },
      ),
    ).toBe(true)
    expect(
      scopesEqual(
        { type: "run", id: "r-1" },
        { type: "run", id: "r-2" },
      ),
    ).toBe(false)
    expect(
      scopesEqual(
        { type: "run", id: "r-1" },
        { type: "workflow", id: "r-1" },
      ),
    ).toBe(false)
  })
})

describe("pinned scope localStorage", () => {
  it("round-trips a pinned scope per (user, workspace)", () => {
    writePinnedScope("user-A", "ws-1", { type: "workflow", id: "wf-1" })
    expect(readPinnedScope("user-A", "ws-1")).toEqual({
      type: "workflow",
      id: "wf-1",
    })
    // Different workspace — no leak.
    expect(readPinnedScope("user-A", "ws-2")).toBeNull()
    // Different user — no leak.
    expect(readPinnedScope("user-B", "ws-1")).toBeNull()
  })

  it("clearPinnedScope removes the entry", () => {
    writePinnedScope("user-A", "ws-1", { type: "run", id: "run-1" })
    clearPinnedScope("user-A", "ws-1")
    expect(readPinnedScope("user-A", "ws-1")).toBeNull()
  })

  it("returns null when user or workspace is missing", () => {
    expect(readPinnedScope(null, "ws-1")).toBeNull()
    expect(readPinnedScope("user-A", null)).toBeNull()
  })
})
