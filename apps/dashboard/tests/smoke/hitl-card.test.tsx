import { describe, expect, it, vi } from "vitest"
import { fireEvent, render, screen } from "@testing-library/react"
import { HitlCard } from "@/components/chat/HitlCard"
import type { HitlCard as HitlCardSpec } from "@/components/chat/hitl-types"
import { findMcpTool, mcpToolBindings } from "@/lib/mcp-tools"

describe("HitlCard — constructive", () => {
  const card: HitlCardSpec = {
    kind: "constructive",
    title: "Create workflow · Auto-merge bot",
    summary: "3 stages",
    body: {
      fields: [
        { label: "Name", value: "Auto-merge bot", editable: true, key: "name" },
        { label: "Repo", value: "onsager-ai/onsager", editable: false },
        { label: "Stages", value: "3", editable: false },
      ],
    },
    commit: { label: "Create workflow", intent: "primary" },
    reject: { label: "Reject" },
  }

  it("renders editable fields with a tool-defined commit label", () => {
    render(
      <HitlCard
        card={card}
        state="pending"
        onCommit={() => {}}
        onReject={() => {}}
      />,
    )
    expect(screen.getByRole("button", { name: /Create workflow/i })).toBeTruthy()
    expect(screen.getByRole("button", { name: /^Reject$/i })).toBeTruthy()
    expect(
      (screen.getByLabelText("Name") as HTMLInputElement).value,
    ).toBe("Auto-merge bot")
  })

  it("commit fires with the user's edited values", () => {
    const onCommit = vi.fn()
    render(
      <HitlCard
        card={card}
        state="pending"
        onCommit={onCommit}
        onReject={() => {}}
      />,
    )
    fireEvent.change(screen.getByLabelText("Name"), {
      target: { value: "Renamed bot" },
    })
    fireEvent.click(screen.getByRole("button", { name: /Create workflow/i }))
    expect(onCommit).toHaveBeenCalledWith({ name: "Renamed bot" })
  })

  it("reject discards without firing commit", () => {
    const onCommit = vi.fn()
    const onReject = vi.fn()
    render(
      <HitlCard
        card={card}
        state="pending"
        onCommit={onCommit}
        onReject={onReject}
      />,
    )
    fireEvent.click(screen.getByRole("button", { name: /^Reject$/i }))
    expect(onReject).toHaveBeenCalled()
    expect(onCommit).not.toHaveBeenCalled()
  })

  it("collapses to a one-line confirmation after commit", () => {
    render(
      <HitlCard
        card={card}
        state="committed"
        onCommit={() => {}}
        onReject={() => {}}
      />,
    )
    expect(screen.queryByRole("button", { name: /Create workflow/i })).toBeNull()
    expect(screen.getByText(/Committed:/)).toBeTruthy()
  })
})

describe("HitlCard — diff", () => {
  const card: HitlCardSpec = {
    kind: "diff",
    title: "Edit workflow · auto-merge",
    summary: "1 field modified",
    body: {
      before: { active: "true" },
      after: { active: "false" },
    },
    commit: { label: "Apply changes", intent: "primary" },
    reject: { label: "Discard" },
  }

  it("highlights modified rows with +/-/~ markers", () => {
    render(
      <HitlCard
        card={card}
        state="pending"
        onCommit={() => {}}
        onReject={() => {}}
      />,
    )
    // Modified row shows both the before (struck-through) and after values.
    expect(screen.getByText("true")).toBeTruthy()
    expect(screen.getByText("false")).toBeTruthy()
    expect(screen.getByText(/~ active/i)).toBeTruthy()
  })
})

describe("HitlCard — destructive", () => {
  const irreversibleCard: HitlCardSpec = {
    kind: "destructive",
    title: "Cancel run · art_123",
    body: { info: "Aborts the in-flight run and archives the artifact." },
    sideEffects: [
      "Sets artifacts.state = 'archived'",
      "Emits `artifact.archived` on the spine",
    ],
    reversibility: "Irreversible at the artifact level.",
    confirmTyping: {
      promptLabel: "Type the artifact id to confirm",
      expectedValue: "art_123",
    },
    commit: { label: "Cancel run art_123", intent: "destructive" },
    reject: { label: "Keep running" },
  }

  it("disables commit until the user types the expected value", () => {
    render(
      <HitlCard
        card={irreversibleCard}
        state="pending"
        onCommit={() => {}}
        onReject={() => {}}
      />,
    )
    const commit = screen.getByRole("button", {
      name: /Cancel run art_123/i,
    }) as HTMLButtonElement
    expect(commit.disabled).toBe(true)
    fireEvent.change(
      screen.getByLabelText("Type the artifact id to confirm"),
      { target: { value: "art_123" } },
    )
    expect(commit.disabled).toBe(false)
  })

  it("surfaces side-effects and reversibility copy", () => {
    render(
      <HitlCard
        card={irreversibleCard}
        state="pending"
        onCommit={() => {}}
        onReject={() => {}}
      />,
    )
    expect(screen.getByText(/Sets artifacts.state/)).toBeTruthy()
    expect(screen.getByText(/Irreversible/)).toBeTruthy()
  })

  it("a reversible destructive card needs no type-to-confirm", () => {
    const reversibleCard: HitlCardSpec = {
      kind: "destructive",
      title: "Pause workflow",
      body: { info: "Pauses the workflow's trigger." },
      reversibility: "Reversible — resume any time.",
      commit: { label: "Pause workflow", intent: "destructive" },
      reject: { label: "Keep active" },
    }
    render(
      <HitlCard
        card={reversibleCard}
        state="pending"
        onCommit={() => {}}
        onReject={() => {}}
      />,
    )
    const commit = screen.getByRole("button", {
      name: /Pause workflow/i,
    }) as HTMLButtonElement
    expect(commit.disabled).toBe(false)
  })
})

describe("mcp-tools registry — HitlCard coverage", () => {
  it("every mutation tool builds a card; every read-only tool renders an info block", () => {
    for (const b of mcpToolBindings()) {
      if (b.category === "read_only") {
        expect(b.buildCard, `${b.name} should NOT build a card`).toBeUndefined()
        expect(b.renderInfo, `${b.name} needs renderInfo`).toBeDefined()
      } else {
        expect(b.buildCard, `${b.name} needs buildCard`).toBeDefined()
        // Mutation tools may also expose renderInfo, but it's optional.
      }
    }
  })

  it("includes the full v1 tool surface", () => {
    for (const expected of [
      "propose_workflow",
      "run_workflow",
      "edit_workflow",
      "schedule_workflow",
      "list_workflows",
      "list_runs",
      "cancel_run",
      "inspect_run",
      "get_stage_logs",
      "get_artifact",
      "propose_remediation",
    ]) {
      expect(findMcpTool(expected), `missing binding for ${expected}`).toBeDefined()
    }
  })
})
