import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { SessionTable } from "@/components/sessions/SessionTable";
import { mockSessions } from "../helpers/mock-api";

function renderWithRouter(ui: React.ReactElement) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    <QueryClientProvider client={queryClient}>
      <MemoryRouter>{ui}</MemoryRouter>
    </QueryClientProvider>,
  );
}

describe("SessionTable", () => {
  it("renders empty state when no sessions", () => {
    renderWithRouter(<SessionTable sessions={[]} />);
    expect(screen.getByText("No sessions yet")).toBeInTheDocument();
  });

  it("renders session IDs (truncated to 8 chars)", () => {
    const sessions = mockSessions(3);
    renderWithRouter(<SessionTable sessions={sessions} />);

    // Each session ID appears twice (mobile card + desktop table)
    for (const s of sessions) {
      const truncated = s.id.slice(0, 8);
      const els = screen.getAllByText(truncated);
      expect(els.length).toBeGreaterThanOrEqual(1);
    }
  });

  it("renders session prompts", () => {
    const sessions = mockSessions(2);
    renderWithRouter(<SessionTable sessions={sessions} />);

    // Prompts appear in both mobile and desktop views
    for (const s of sessions) {
      const prompt = s.prompt.slice(0, 80);
      const els = screen.getAllByText(prompt);
      expect(els.length).toBeGreaterThanOrEqual(1);
    }
  });

  it("renders links to session detail page", () => {
    const sessions = mockSessions(1);
    renderWithRouter(<SessionTable sessions={sessions} />);

    const links = screen.getAllByRole("link");
    const detailLink = links.find((l) =>
      l.getAttribute("href")?.includes(`/sessions/${sessions[0].id}`),
    );
    expect(detailLink).toBeDefined();
  });

  it("renders state badges for each session", () => {
    const sessions = mockSessions(1);
    renderWithRouter(<SessionTable sessions={sessions} />);

    // First session is "pending" state
    const badges = screen.getAllByText("Pending");
    expect(badges.length).toBeGreaterThanOrEqual(1);
  });
});
