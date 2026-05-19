import { describe, it, expect } from "vitest"
import { render, screen, waitFor } from "@testing-library/react"
import { MemoryRouter, Routes, Route, Navigate, useLocation } from "react-router-dom"

// Per spec #398 the bare path `/` is the universal landing and redirects
// unconditionally to `/chat`. ChatPage resolves the user's workspace
// context internally (last-used or zero-state FTUE), so the redirect
// no longer needs to read membership state or `localStorage`.
//
// The "bounce to first workspace" machinery was deleted in spec #403;
// this test pins the post-demolition invariant. We replicate the App.tsx
// redirect row here rather than rendering the full App to keep the test
// scope tight (no AuthProvider, no lazy chunks).

function BarePathRoute() {
  return <Navigate to="/chat" replace />
}

function LocationProbe() {
  const loc = useLocation()
  return <div data-testid="location">{loc.pathname}</div>
}

function renderRoute(initial: string) {
  return render(
    <MemoryRouter initialEntries={[initial]}>
      <Routes>
        <Route path="/" element={<BarePathRoute />} />
        <Route path="/chat" element={<LocationProbe />} />
        <Route path="/workspaces/:workspace/*" element={<LocationProbe />} />
      </Routes>
    </MemoryRouter>,
  )
}

describe("App-level bare-path redirect (#398, #403)", () => {
  it("bare `/` redirects to /chat regardless of workspace state", async () => {
    renderRoute("/")
    await waitFor(() =>
      expect(screen.getByTestId("location")).toHaveTextContent("/chat"),
    )
  })

  it("does not bounce to a workspace overview", async () => {
    renderRoute("/")
    await waitFor(() =>
      expect(screen.getByTestId("location")).toHaveTextContent("/chat"),
    )
    expect(screen.getByTestId("location").textContent).not.toMatch(
      /^\/workspaces\//,
    )
  })
})
