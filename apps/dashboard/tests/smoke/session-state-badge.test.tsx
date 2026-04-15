import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import { SessionStateBadge } from "@/components/sessions/SessionStateBadge";

describe("SessionStateBadge", () => {
  const states = [
    { state: "pending", label: "Pending" },
    { state: "dispatched", label: "Dispatched" },
    { state: "running", label: "Running" },
    { state: "waiting_input", label: "Waiting Input" },
    { state: "done", label: "Done" },
    { state: "failed", label: "Failed" },
  ];

  for (const { state, label } of states) {
    it(`renders "${label}" for state "${state}"`, () => {
      render(<SessionStateBadge state={state} />);
      expect(screen.getByText(label)).toBeInTheDocument();
    });
  }

  it("falls back to Pending for unknown state", () => {
    render(<SessionStateBadge state="unknown_state" />);
    expect(screen.getByText("Pending")).toBeInTheDocument();
  });
});
