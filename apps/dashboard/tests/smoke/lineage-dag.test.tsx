import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { LineageDAG } from "@/components/factory/LineageDAG";
import { buildLanes } from "@/components/factory/lineage-dag-utils";
import type { ArtifactDetail, SpineEvent } from "@/lib/api";

function baseArtifact(overrides: Partial<ArtifactDetail> = {}): ArtifactDetail {
  return {
    id: "art_01",
    kind: "code",
    name: "demo",
    owner: "marvin",
    state: "in_progress",
    current_version: 1,
    created_at: "2026-04-01T00:00:00Z",
    updated_at: "2026-04-01T00:00:00Z",
    created_by: "dashboard",
    versions: [],
    vertical_lineage: [],
    ...overrides,
  };
}

describe("LineageDAG", () => {
  it("shows empty-state copy when there are no versions", () => {
    render(
      <MemoryRouter>
        <LineageDAG artifact={baseArtifact()} />
      </MemoryRouter>,
    );
    expect(
      screen.getByText(/No shaping runs yet/i),
    ).toBeInTheDocument();
  });

  it("renders one lane per version with its session id", () => {
    const artifact = baseArtifact({
      current_version: 1,
      versions: [
        {
          version: 1,
          content_ref_uri: "git://test",
          content_ref_checksum: null,
          change_summary: "first shape",
          created_by_session: "sess_abcdef12345678",
          parent_version: null,
          created_at: "2026-04-01T01:00:00Z",
        },
      ],
    });
    render(
      <MemoryRouter>
        <LineageDAG artifact={artifact} />
      </MemoryRouter>,
    );
    // Several elements repeat between the SVG and the run list.
    expect(screen.getAllByText("v1").length).toBeGreaterThanOrEqual(1);
    expect(screen.getAllByText(/first shape/).length).toBeGreaterThanOrEqual(1);
    expect(screen.getAllByText(/sess_abcdef1234/).length).toBeGreaterThanOrEqual(
      1,
    );
  });
});

describe("LineageDAG.buildLanes", () => {
  it("groups events by request_id", () => {
    const events: SpineEvent[] = [
      {
        id: 1,
        stream_id: "forge:art_01",
        stream_type: "forge",
        event_type: "forge.shaping_dispatched",
        data: { request_id: "r1", artifact_id: "art_01" },
        actor: "forge",
        created_at: "2026-04-01T00:00:00Z",
      },
      {
        id: 2,
        stream_id: "forge:art_01",
        stream_type: "forge",
        event_type: "forge.gate_verdict",
        data: { request_id: "r1", verdict: "Allow" },
        actor: "synodic",
        created_at: "2026-04-01T00:00:01Z",
      },
      {
        id: 3,
        stream_id: "forge:art_01",
        stream_type: "forge",
        event_type: "forge.shaping_dispatched",
        data: { request_id: "r2", artifact_id: "art_01" },
        actor: "forge",
        created_at: "2026-04-01T00:10:00Z",
      },
    ];
    const lanes = buildLanes(events);
    expect(lanes.size).toBe(2);
    expect(lanes.get("r1")?.length).toBe(2);
    expect(lanes.get("r2")?.length).toBe(1);
  });

  it("ignores events with no request_id", () => {
    const lanes = buildLanes([
      {
        id: 1,
        stream_id: "s",
        stream_type: "forge",
        event_type: "forge.idle_tick",
        data: {},
        actor: "forge",
        created_at: "2026-04-01T00:00:00Z",
      },
    ]);
    expect(lanes.size).toBe(0);
  });
});
