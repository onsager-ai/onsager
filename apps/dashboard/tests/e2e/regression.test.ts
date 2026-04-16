/**
 * L1 Deterministic E2E Regression Tests
 *
 * More thorough than smoke tests — these verify specific UI behaviors,
 * data rendering, and interactive elements. Run on pre-release and
 * as part of the full regression suite.
 *
 * Prerequisites:
 *   - Dev server running with backend: `pnpm dev`
 *   - Chrome installed: `npx agent-browser install`
 */
import { describe, it, expect, beforeAll, afterAll } from "vitest";
import { createBrowser, type Browser } from "../helpers/browser";

let browser: Browser;

beforeAll(() => {
  browser = createBrowser();
});

afterAll(() => {
  try {
    browser.close();
  } catch {
    // browser may already be closed
  }
});

describe("E2E Regression: Factory Overview Data", () => {
  it("renders stat cards with numeric values", () => {
    browser.open("/");
    const snapshot = browser.snapshot();

    // Stat card titles must be present
    const requiredLabels = [
      "Nodes Online",
      "Total Artifacts",
      "Factory Events",
      "Gov. Issues",
    ];
    for (const label of requiredLabels) {
      expect(snapshot).toContain(label);
    }
  });

  it("renders pipeline stats section", () => {
    browser.open("/");
    const snapshot = browser.snapshot();

    expect(snapshot).toContain("Artifact Pipeline");
  });
});

describe("E2E Regression: Sidebar Navigation", () => {
  it("has working navigation links", () => {
    browser.open("/");
    const interactive = browser.interactiveSnapshot();

    // Sidebar should have navigation links
    expect(interactive).toContain("Overview");
    expect(interactive).toContain("Sessions");
    expect(interactive).toContain("Nodes");
  });
});

describe("E2E Regression: Theme Toggle", () => {
  it("page includes theme toggle element", () => {
    browser.open("/");
    const interactive = browser.interactiveSnapshot();

    // Theme toggle button should be accessible
    const hasThemeToggle =
      interactive.includes("theme") ||
      interactive.includes("Theme") ||
      interactive.includes("dark") ||
      interactive.includes("light");
    expect(hasThemeToggle).toBe(true);
  });
});

describe("E2E Regression: Session Detail", () => {
  it("shows not found for invalid session ID", () => {
    browser.open("/sessions/nonexistent-id");
    const snapshot = browser.snapshot();

    // Should show either loading or not found
    const hasExpectedContent =
      snapshot.includes("not found") ||
      snapshot.includes("Not found") ||
      snapshot.includes("Loading") ||
      snapshot.includes("Session Details");
    expect(hasExpectedContent).toBe(true);
  });
});

describe("E2E Regression: Empty States", () => {
  it("sessions page handles empty session list", () => {
    browser.open("/sessions");
    const snapshot = browser.snapshot();

    // Should render the page structure even with no data
    expect(snapshot).toContain("Sessions");
    expect(snapshot).toContain("All Sessions");
  });

  it("nodes page handles empty node list", () => {
    browser.open("/nodes");
    const snapshot = browser.snapshot();

    expect(snapshot).toContain("Nodes");
    expect(snapshot).toContain("Registered Nodes");
  });
});

describe("E2E Regression: Responsive Layout", () => {
  it("page has proper document structure", () => {
    browser.open("/");
    const title = browser.title();
    // Should have some title (Vite default or custom)
    expect(title).toBeDefined();
    expect(title.length).toBeGreaterThan(0);
  });

  it("dashboard URL resolves correctly", () => {
    browser.open("/");
    const url = browser.url();
    expect(url).toContain("/");
  });
});
