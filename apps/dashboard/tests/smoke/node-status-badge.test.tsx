import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import { NodeStatusBadge } from "@/components/nodes/NodeStatusBadge";

describe("NodeStatusBadge", () => {
  it("renders Online for online status", () => {
    render(<NodeStatusBadge status="online" />);
    expect(screen.getByText("Online")).toBeInTheDocument();
  });

  it("renders Offline for offline status", () => {
    render(<NodeStatusBadge status="offline" />);
    expect(screen.getByText("Offline")).toBeInTheDocument();
  });

  it("renders Draining for draining status", () => {
    render(<NodeStatusBadge status="draining" />);
    expect(screen.getByText("Draining")).toBeInTheDocument();
  });

  it("falls back to Offline for unknown status", () => {
    render(<NodeStatusBadge status="banana" />);
    expect(screen.getByText("Offline")).toBeInTheDocument();
  });
});
