/**
 * L1 Deterministic E2E Smoke Tests
 *
 * Uses agent-browser to verify critical UI paths against a running stack
 * (the stiglab Docker image, which serves the dashboard + backend on :3000).
 *
 * Prerequisites:
 *   - Stack running at ONSAGER_TEST_URL (default http://localhost:3000)
 *   - Chrome installed: `pnpm exec agent-browser install --with-deps`
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

describe("E2E Smoke: Navigation", () => {
  it("loads the factory overview", () => {
    browser.open("/");
    const snapshot = browser.snapshot();
    expect(snapshot).toContain("Factory");
  });

  it("navigates to Sessions page", () => {
    browser.open("/sessions");
    const snapshot = browser.snapshot();
    expect(snapshot).toContain("Sessions");
  });

  it("navigates to Nodes page", () => {
    browser.open("/nodes");
    const snapshot = browser.snapshot();
    expect(snapshot).toContain("Nodes");
  });

  it("navigates to Artifacts page", () => {
    browser.open("/artifacts");
    const snapshot = browser.snapshot();
    expect(snapshot).toContain("Artifacts");
  });

  it("navigates to Governance page", () => {
    browser.open("/governance");
    const snapshot = browser.snapshot();
    expect(snapshot).toContain("Governance");
  });

  it("navigates to Settings page", () => {
    browser.open("/settings");
    const snapshot = browser.snapshot();
    expect(snapshot).toContain("Settings");
  });
});

describe("E2E Smoke: Factory Overview", () => {
  it("renders overview stat cards", () => {
    browser.open("/");
    const snapshot = browser.snapshot();

    expect(snapshot).toContain("Nodes Online");
    expect(snapshot).toContain("Active Sessions");
    expect(snapshot).toContain("Completed");
  });
});

describe("E2E Smoke: Sessions Page", () => {
  it("shows sessions heading", () => {
    browser.open("/sessions");
    const snapshot = browser.snapshot();
    expect(snapshot).toContain("Sessions");
  });
});

describe("E2E Smoke: Nodes Page", () => {
  it("shows nodes heading", () => {
    browser.open("/nodes");
    const snapshot = browser.snapshot();
    expect(snapshot).toContain("Nodes");
  });
});
