import { describe, it, expect } from "vitest"
import { render } from "@testing-library/react"
import { ArtifactFlowOverview } from "@/components/factory/workflows/ArtifactFlowOverview"
import type { WorkflowStage } from "@/lib/api"

// Issue #100/#104 regression: the pre-#104 flow strip rendered input +
// output artifact badges per stage, which duplicated "PR" four times on
// the Governed pipeline preset (`Issue → PR → PR → PR → PR`). Post-#104
// the strip renders one pill per gate — no artifact duplication.
describe("ArtifactFlowOverview — gate-only strip (#104)", () => {
  const stages: WorkflowStage[] = [
    {
      id: "s1",
      name: "Spec → PR",
      gate_kind: "agent-session",
      artifact_kind: "Issue",
      config: {},
    },
    {
      id: "s2",
      name: "CI check",
      gate_kind: "external-check",
      artifact_kind: "PR",
      config: {},
    },
    {
      id: "s3",
      name: "Synodic gate",
      gate_kind: "governance",
      artifact_kind: "PR",
      config: {},
    },
    {
      id: "s4",
      name: "Merge approval",
      gate_kind: "manual-approval",
      artifact_kind: "PR",
      config: {},
    },
  ]

  it("renders one pill per stage with no duplicate artifact badges", () => {
    const { container } = render(
      <ArtifactFlowOverview triggerLabel="spec" stages={stages} />,
    )
    const strip = container.querySelector(
      '[data-testid="workflow-flow-strip"]',
    )
    expect(strip).toBeTruthy()
    // The four gates we supplied, no more — prior to #104 this would have
    // been 4 gates + 5 artifact pills.
    const pills = strip?.querySelectorAll("span.rounded-full") ?? []
    // Trigger pill (1) + one per stage (4) = 5.
    expect(pills.length).toBe(5)
  })

  it("does not render any artifact-kind badge text", () => {
    const { queryByText } = render(
      <ArtifactFlowOverview triggerLabel="spec" stages={stages} />,
    )
    // Short labels used to appear as standalone pills; they must not.
    expect(queryByText("Issue")).toBeNull()
    expect(queryByText("PR")).toBeNull()
  })
})
