import { describe, it, expect } from "vitest";
import type { GitHubAppInstallation } from "@/lib/api";
import {
  documentToCreateRequest,
  emptyDocument,
  type WorkflowDocument,
} from "@/components/workflows/workflow-draft";

// The portal `validate_create_body` is the source of truth for the wire
// shape. These tests pin the UI → backend adapter against its contract:
// a structured `trigger: TriggerKind` (ADR 0024 / spec #509), numeric
// install_id resolved from the install record, `active`, and stage params
// that carry the UI-only name alongside any gate config.
describe("documentToCreateRequest", () => {
  const installation: GitHubAppInstallation = {
    id: "inst_abc",
    workspace_id: "t_1",
    install_id: 12345,
    account_login: "onsager-ai",
    account_type: "organization",
    created_at: "2026-04-22T00:00:00Z",
  };

  const draft = (): WorkflowDocument => ({
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
        config: { agent_profile: "default" },
      },
    ],
  });

  it("resolves the numeric GitHub install_id from the installations list", () => {
    const out = documentToCreateRequest(draft(), [installation], "t_1", true);
    expect(out.install_id).toBe(12345);
    expect(typeof out.install_id).toBe("number");
  });

  it("assembles a structured github_issue_webhook trigger", () => {
    const out = documentToCreateRequest(draft(), [installation], "t_1", true);
    expect(out).toMatchObject({
      workspace_id: "t_1",
      name: "Issue → PR",
      trigger: {
        kind: "github_issue_webhook",
        repo: "onsager-ai/onsager",
        label: "factory",
      },
      active: true,
    });
    // Structured trigger, not flat fields — and no stray `activate` key.
    expect("trigger_kind" in out).toBe(false);
    expect("repo_owner" in out).toBe(false);
    expect("activate" in out).toBe(false);
  });

  it("maps stages to { gate_kind, params } and carries the UI name in params", () => {
    const out = documentToCreateRequest(draft(), [installation], "t_1", true);
    expect(out.stages).toEqual([
      {
        gate_kind: "agent-session",
        params: {
          agent_profile: "default",
          name: "Spec → PR",
        },
      },
    ]);
  });

  it("passes activate=false through as active=false", () => {
    const out = documentToCreateRequest(draft(), [installation], "t_1", false);
    expect(out.active).toBe(false);
  });

  it("throws when workspace_id is blank", () => {
    expect(() =>
      documentToCreateRequest(draft(), [installation], "  ", true),
    ).toThrow(/workspace_id/);
  });

  it("throws when the selected install isn't in the list", () => {
    expect(() => documentToCreateRequest(draft(), [], "t_1", true)).toThrow(
      /install not found/,
    );
  });

  it("throws when a GitHub trigger isn't ready (missing label)", () => {
    // `emptyDocument()` now defaults to Manual (always ready, #572), so build
    // an explicit half-filled GitHub-webhook trigger to exercise the gate.
    const d: WorkflowDocument = {
      ...emptyDocument(),
      name: "Issue → PR",
      trigger: {
        kind_tag: "github_issue_webhook",
        install_id: "inst_abc",
        repo_owner: "onsager-ai",
        repo_name: "onsager",
        label: "",
        manual_name: "",
      },
      stages: [
        { id: "s1", name: "Spec → PR", gate_kind: "agent-session", config: {} },
      ],
    };
    expect(() =>
      documentToCreateRequest(d, [installation], "t_1", true),
    ).toThrow(/install, repo, and label/);
  });
});
