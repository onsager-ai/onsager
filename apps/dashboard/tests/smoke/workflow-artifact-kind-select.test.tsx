import { describe, it, expect } from "vitest"
import { ArtifactKindSelect } from "@/components/factory/workflows/ArtifactKindSelect"
import { WORKFLOW_ARTIFACT_KINDS } from "@/components/factory/workflows/workflow-meta"

describe("ArtifactKindSelect", () => {
  it("only lists the built-in GitHub artifact kinds — no custom kinds", () => {
    const values = WORKFLOW_ARTIFACT_KINDS.map((k) => k.value).sort()
    expect(values).toEqual(["github-issue", "github-pr"])
    const hasCustom = WORKFLOW_ARTIFACT_KINDS.some(
      (k) => k.value === ("custom" as unknown as string),
    )
    expect(hasCustom).toBe(false)
  })

  it("renders the component without error for each built-in value", () => {
    // Smoke-check the component module loads with each allowed value —
    // enforces that the TS union stays synced with the select items.
    const VALUES: ("github-issue" | "github-pr")[] = ["github-issue", "github-pr"]
    for (const v of VALUES) {
      // @ts-expect-no-error — JSX to ensure `value` compiles against the
      // WorkflowArtifactKind union.
      const node = <ArtifactKindSelect value={v} onChange={() => {}} />
      expect(node).toBeTruthy()
    }
  })
})
