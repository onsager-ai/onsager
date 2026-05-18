import { describe, it, expect, vi } from "vitest"
import { render, screen, fireEvent } from "@testing-library/react"
import {
  TEMPLATES,
  getTemplate,
  templateToDocument,
  type FtueTemplate,
} from "@/lib/templates"
import { TemplateGallery } from "@/components/chat/TemplateGallery"
import { makeDraft } from "@/lib/drafts"

// The valid gate-kind set comes from `WorkflowGateKind` (apps/dashboard/src/
// lib/api/types.ts) — these are the only stage discriminants the wire
// accepts. New templates must use one of these; otherwise the DAG would
// reference a substrate-unknown gate.
const VALID_GATE_KINDS = new Set([
  "agent-session",
  "external-check",
  "governance",
  "manual-approval",
])

// Registered workflow artifact kinds from `crates/onsager-registry/src/
// catalog.rs` (BUILTIN_WORKFLOW_KINDS). A template's stages and
// primary_artifact_kind must match one of these — otherwise the workflow
// can't be persisted and the spec's acceptance criterion #4 fails.
const REGISTERED_ARTIFACT_KINDS = new Set(["Issue", "PR", "Deployment", "Session"])

// Trigger taxonomy from `crates/onsager-registry/src/triggers.rs`. Per
// spec #406's Substrate touchpoints, templates must reshape to a coarser
// existing trigger rather than introducing a new TriggerKind.
const REGISTERED_TRIGGER_KINDS = new Set([
  "github_issue_webhook",
  "github_pull_request_closed",
  "github_workflow_run_completed",
  "telegram_webhook",
  "cron",
  "delay",
  "interval",
  "spine_event",
  "pg_notify",
  "outbox_row",
  "manual",
  "replay",
])

describe("FTUE template manifest (#406)", () => {
  it("ships six templates", () => {
    expect(TEMPLATES).toHaveLength(6)
  })

  it("contains the locked template ids", () => {
    const ids = TEMPLATES.map((t) => t.id)
    expect(ids).toEqual([
      "auto-merge-on-green",
      "spec-compliance-gate",
      "labeled-issue-summary",
      "release-notes-from-merged-prs",
      "security-review-on-pr",
      "onsager-dogfood",
    ])
  })

  it("is dominantly class D", () => {
    const classD = TEMPLATES.filter((t) => t.scenario_class === "D").length
    expect(classD).toBe(TEMPLATES.length)
  })

  it.each(TEMPLATES)(
    "template $id stages reference valid gate kinds",
    (template: FtueTemplate) => {
      for (const stage of template.stages) {
        expect(VALID_GATE_KINDS.has(stage.gate_kind)).toBe(true)
      }
    },
  )

  it.each(TEMPLATES)(
    "template $id stage artifact kinds are registered",
    (template: FtueTemplate) => {
      for (const stage of template.stages) {
        expect(REGISTERED_ARTIFACT_KINDS.has(stage.artifact_kind)).toBe(true)
      }
    },
  )

  it.each(TEMPLATES)(
    "template $id primary_artifact_kind is registered",
    (template: FtueTemplate) => {
      expect(REGISTERED_ARTIFACT_KINDS.has(template.primary_artifact_kind)).toBe(true)
    },
  )

  it.each(TEMPLATES)(
    "template $id trigger_kind is registered",
    (template: FtueTemplate) => {
      expect(REGISTERED_TRIGGER_KINDS.has(template.trigger_kind)).toBe(true)
    },
  )

  it.each(TEMPLATES)(
    "template $id carries a non-empty factory_framing string",
    (template: FtueTemplate) => {
      expect(template.factory_framing.length).toBeGreaterThan(0)
    },
  )

  it("locked factory_framing strings match #406 / #408 spec", () => {
    const byId = Object.fromEntries(TEMPLATES.map((t) => [t.id, t.factory_framing]))
    expect(byId["auto-merge-on-green"]).toBe(
      "A QC checkpoint that lets clean PRs ship themselves.",
    )
    expect(byId["spec-compliance-gate"]).toBe(
      "Refuses any product not tied to a blueprint.",
    )
    expect(byId["labeled-issue-summary"]).toBe(
      "Sends incoming orders through an agent station for triage.",
    )
    expect(byId["release-notes-from-merged-prs"]).toBe(
      "Weekly inventory report from the production line.",
    )
    expect(byId["security-review-on-pr"]).toBe(
      "Human inspection station for the sensitive line.",
    )
    expect(byId["onsager-dogfood"]).toBe(
      "The production line that builds Onsager itself.",
    )
  })
})

describe("templateToDocument", () => {
  it("leaves trigger install/repo blank — binding (#402) fills them", () => {
    const template = getTemplate("auto-merge-on-green")!
    const doc = templateToDocument(template)
    expect(doc.trigger.install_id).toBe("")
    expect(doc.trigger.repo_owner).toBe("")
    expect(doc.trigger.repo_name).toBe("")
  })

  it("projects stages with the template's gate kinds and names", () => {
    const template = getTemplate("labeled-issue-summary")!
    const doc = templateToDocument(template)
    expect(doc.stages).toHaveLength(template.stages.length)
    expect(doc.stages[0].gate_kind).toBe(template.stages[0].gate_kind)
    expect(doc.stages[0].name).toBe(template.stages[0].name)
  })

  it("wraps into a WorkflowDraft carrying source + template_id (#404)", () => {
    // The outer WorkflowDraft is where activation instrumentation reads
    // provenance from — templateToDocument supplies the inner document,
    // makeDraft composes the outer record.
    const template = getTemplate("auto-merge-on-green")!
    const draft = makeDraft(
      "user_test",
      "template",
      templateToDocument(template),
      template.name,
      template.id,
    )
    expect(draft.source).toBe("template")
    expect(draft.template_id).toBe("auto-merge-on-green")
  })
})

describe("TemplateGallery", () => {
  it("renders one card per template", () => {
    render(<TemplateGallery onPick={() => {}} />)
    for (const t of TEMPLATES) {
      expect(screen.getByText(t.name)).toBeTruthy()
      expect(screen.getByText(t.factory_framing)).toBeTruthy()
    }
  })

  it("fires onPick when a card is clicked", () => {
    const onPick = vi.fn()
    render(<TemplateGallery onPick={onPick} />)
    fireEvent.click(screen.getByText("Auto-merge on green"))
    expect(onPick).toHaveBeenCalledTimes(1)
    expect(onPick.mock.calls[0][0].id).toBe("auto-merge-on-green")
  })

  it("highlights the selected template", () => {
    const { container } = render(
      <TemplateGallery onPick={() => {}} selectedId="onsager-dogfood" />,
    )
    const selected = container.querySelector('[data-selected="true"]')
    expect(selected).toBeTruthy()
    expect(selected?.textContent).toContain("Onsager dogfood")
  })
})
