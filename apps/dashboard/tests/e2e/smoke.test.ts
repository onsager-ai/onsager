/**
 * L1 Deterministic E2E Smoke Tests
 *
 * Uses agent-browser to verify critical UI paths against a running stack
 * (the stiglab Docker image, which serves the dashboard + backend on :3000).
 *
 * Assertions check both `browser.url()` (confirm routing actually happened)
 * and page-unique content (confirm the page itself rendered). This avoids
 * false positives from sidebar labels like "Sessions" / "Nodes" which
 * appear on every authenticated page.
 *
 * Prerequisites:
 *   - Stack running at ONSAGER_TEST_URL (default http://localhost:3000)
 *   - `agent-browser` on PATH (install globally: `npm i -g agent-browser`,
 *     or add as a devDependency) with Chrome prepared via
 *     `agent-browser install --with-deps`.
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

// Each expected route: path, unique text only that page renders, and a URL
// fragment to confirm the router landed on it.
const routes = [
  { path: "/", unique: "Artifact Pipeline", urlEndsWith: "/" },
  { path: "/sessions", unique: "All Sessions", urlEndsWith: "/sessions" },
  { path: "/nodes", unique: "Registered Nodes", urlEndsWith: "/nodes" },
  { path: "/artifacts", unique: "All Artifacts", urlEndsWith: "/artifacts" },
  { path: "/governance", unique: "Governance", urlEndsWith: "/governance" },
  { path: "/settings", unique: "Credentials", urlEndsWith: "/settings" },
] as const;

describe("E2E Smoke: Navigation", () => {
  for (const route of routes) {
    it(`lands on ${route.path} and renders page-unique content`, () => {
      browser.open(route.path);
      browser.waitForText(route.unique);
      const url = browser.url();
      expect(url.endsWith(route.urlEndsWith)).toBe(true);
      const snapshot = browser.snapshot();
      expect(snapshot).toContain(route.unique);
    });
  }
});

describe("E2E Smoke: Factory Overview", () => {
  it("renders overview stat cards", () => {
    browser.open("/");
    browser.waitForText("Artifact Pipeline");
    const snapshot = browser.snapshot();

    expect(snapshot).toContain("Nodes Online");
    expect(snapshot).toContain("Active Sessions");
    expect(snapshot).toContain("Completed");
  });
});

describe("E2E Smoke: Sessions Page", () => {
  it("shows the All Sessions table", () => {
    browser.open("/sessions");
    browser.waitForText("All Sessions");
    const snapshot = browser.snapshot();
    expect(snapshot).toContain("All Sessions");
  });
});

describe("E2E Smoke: Nodes Page", () => {
  it("shows the Registered Nodes table", () => {
    browser.open("/nodes");
    browser.waitForText("Registered Nodes");
    const snapshot = browser.snapshot();
    expect(snapshot).toContain("Registered Nodes");
  });
});
