import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import { NodeTable } from "@/components/nodes/NodeTable";
import { mockNodes } from "../helpers/mock-api";

describe("NodeTable", () => {
  it("renders empty state when no nodes", () => {
    render(<NodeTable nodes={[]} />);
    expect(screen.getByText("No nodes registered")).toBeInTheDocument();
  });

  it("renders node names", () => {
    const nodes = mockNodes(3);
    render(<NodeTable nodes={nodes} />);

    for (const n of nodes) {
      const els = screen.getAllByText(n.name);
      expect(els.length).toBeGreaterThanOrEqual(1);
    }
  });

  it("renders node hostnames", () => {
    const nodes = mockNodes(2);
    render(<NodeTable nodes={nodes} />);

    for (const n of nodes) {
      const els = screen.getAllByText(n.hostname);
      expect(els.length).toBeGreaterThanOrEqual(1);
    }
  });

  it("renders session capacity as fraction", () => {
    const nodes = mockNodes(3);
    render(<NodeTable nodes={nodes} />);

    for (const n of nodes) {
      const fraction = `${n.active_sessions}/${n.max_sessions}`;
      const els = screen.getAllByText(fraction);
      expect(els.length).toBeGreaterThanOrEqual(1);
    }
  });

  it("renders status badges", () => {
    const nodes = mockNodes(3); // online, offline, draining
    render(<NodeTable nodes={nodes} />);

    // Each status appears at least once (mobile + desktop)
    expect(screen.getAllByText("Online").length).toBeGreaterThanOrEqual(1);
    expect(screen.getAllByText("Offline").length).toBeGreaterThanOrEqual(1);
    expect(screen.getAllByText("Draining").length).toBeGreaterThanOrEqual(1);
  });
});
