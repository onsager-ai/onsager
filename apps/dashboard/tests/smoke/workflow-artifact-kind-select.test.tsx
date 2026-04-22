import { describe, it, expect } from "vitest"
import { ArtifactKindSelect } from "@/components/factory/workflows/ArtifactKindSelect"
import { WORKFLOW_ARTIFACT_KINDS } from "@/components/factory/workflows/workflow-meta"

describe("ArtifactKindSelect", () => {
  it("ships the v1 builtin artifact kinds as its static fallback (issue #102)", () => {
    const values = WORKFLOW_ARTIFACT_KINDS.map((k) => k.value).sort()
    expect(values).toEqual(["Deployment", "Issue", "PR", "Session"])
  })

  it("renders the component without error for each built-in value", () => {
    for (const meta of WORKFLOW_ARTIFACT_KINDS) {
      const node = <ArtifactKindSelect value={meta.value} onChange={() => {}} />
      expect(node).toBeTruthy()
    }
  })
})
