import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import { Overview } from "@/components/dashboard/Overview";
import { mockNodes, mockSessions } from "../helpers/mock-api";

describe("Overview", () => {
  it("renders all four stat cards", () => {
    render(<Overview nodes={mockNodes(3)} sessions={mockSessions(6)} />);

    expect(screen.getByText("Nodes Online")).toBeInTheDocument();
    expect(screen.getByText("Active Sessions")).toBeInTheDocument();
    expect(screen.getByText("Waiting Input")).toBeInTheDocument();
    expect(screen.getByText("Completed")).toBeInTheDocument();
  });

  it("computes correct node counts", () => {
    const nodes = mockNodes(3); // statuses cycle: online, offline, draining
    render(<Overview nodes={nodes} sessions={[]} />);

    // 1 online out of 3
    expect(screen.getByText("1/3")).toBeInTheDocument();
    expect(screen.getByText("2 offline")).toBeInTheDocument();
  });

  it("computes correct active session count", () => {
    // states cycle: pending, dispatched, running, waiting_input, done, failed
    // active = dispatched(1) + running(1) + waiting_input(1) = 3
    const sessions = mockSessions(6);
    render(<Overview nodes={[]} sessions={sessions} />);

    expect(screen.getByText("3")).toBeInTheDocument(); // active sessions
  });

  it("highlights waiting input when count > 0", () => {
    const sessions = mockSessions(6); // includes 1 waiting_input
    const { container } = render(<Overview nodes={[]} sessions={sessions} />);

    const yellowBorderCard = container.querySelector(".border-yellow-500\\/50");
    expect(yellowBorderCard).toBeInTheDocument();
  });

  it("renders with empty data", () => {
    render(<Overview nodes={[]} sessions={[]} />);

    expect(screen.getByText("0/0")).toBeInTheDocument();
    expect(screen.getAllByText("0")).toHaveLength(5); // active, waiting, completed, gov issues, artifacts
  });
});
