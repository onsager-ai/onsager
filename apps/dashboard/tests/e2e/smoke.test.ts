/**
 * L1 Deterministic E2E Smoke Tests
 *
 * These tests use agent-browser to verify critical UI paths against
 * a running dev server. They are deterministic and stable — suitable
 * for regression testing on every PR and pre-release.
 *
 * Prerequisites:
 *   - Dev server running: `pnpm dev` (or at STIGLAB_TEST_URL)
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

describe("E2E Smoke: Navigation", () => {
  it("loads the factory overview page", () => {
    const output = browser.open("/");
    expect(output).toBeDefined();

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
});

describe("E2E Smoke: Factory Overview", () => {
  it("displays overview stat cards", () => {
    browser.open("/");
    const snapshot = browser.snapshot();

    expect(snapshot).toContain("Nodes Online");
    expect(snapshot).toContain("Total Artifacts");
    expect(snapshot).toContain("Factory Events");
  });

  it("displays active artifacts section", () => {
    browser.open("/");
    const snapshot = browser.snapshot();
    expect(snapshot).toContain("Active Artifacts");
  });
});

describe("E2E Smoke: Sessions Page", () => {
  it("shows sessions heading and table", () => {
    browser.open("/sessions");
    const snapshot = browser.snapshot();

    expect(snapshot).toContain("Sessions");
    expect(snapshot).toContain("All Sessions");
  });
});

describe("E2E Smoke: Nodes Page", () => {
  it("shows nodes heading and table", () => {
    browser.open("/nodes");
    const snapshot = browser.snapshot();

    expect(snapshot).toContain("Nodes");
    expect(snapshot).toContain("Registered Nodes");
  });
});
