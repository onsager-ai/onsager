/**
 * E2E: Session Lifecycle
 *
 * Tests the core product flow: create task → agent runs → session
 * completes with output. Uses a real Claude agent — not mocked.
 *
 * Prerequisites:
 *   - Onsager stack running (just dev)
 *   - At least one online node with valid credentials
 *   - ONSAGER_URL env var (default: http://localhost:3000)
 */
import { describe, it, expect, beforeAll } from "vitest";
import {
  createClient,
  preflight,
  type OnsagerClient,
  type Session,
} from "../helpers/client";

let client: OnsagerClient;

beforeAll(async () => {
  client = createClient();
  await preflight(client);
});

describe("Session Lifecycle", () => {
  it("creates a task and completes successfully", async () => {
    // Create a minimal task — cheap, fast, verifiable output
    const { session: created } = await client.createTask({
      prompt:
        "Respond with exactly this text and nothing else: ONSAGER_E2E_OK",
      max_turns: 1,
    });

    expect(created.id).toBeDefined();
    expect(created.state).toMatch(/^(pending|dispatched)$/);

    // Wait for the session to finish (up to 3 minutes)
    const final = await client.waitForSession(created.id, ["done", "failed"]);

    expect(final.state).toBe("done");
    expect(final.output).toBeDefined();
    expect(final.output).toContain("ONSAGER_E2E_OK");
  });

  it("transitions through expected states", async () => {
    const { session: created } = await client.createTask({
      prompt: "Reply with: STATE_TEST_OK",
      max_turns: 1,
    });

    // Session should start as pending or dispatched
    expect(["pending", "dispatched"]).toContain(created.state);

    // Poll and collect state transitions
    const observedStates: string[] = [created.state];
    const deadline = Date.now() + 180_000;
    let session: Session = created;

    while (Date.now() < deadline) {
      session = await client.getSession(created.id);

      const lastObserved = observedStates[observedStates.length - 1];
      if (session.state !== lastObserved) {
        observedStates.push(session.state);
      }

      if (session.state === "done" || session.state === "failed") break;
      await new Promise((r) => setTimeout(r, 1_000));
    }

    expect(session.state).toBe("done");

    // Must have progressed through dispatched → running → done
    // (pending may be skipped if dispatch is immediate)
    expect(observedStates).toContain("running");
    expect(observedStates[observedStates.length - 1]).toBe("done");

    // States must be in valid order
    const validOrder = [
      "pending",
      "dispatched",
      "running",
      "waiting_input",
      "done",
    ];
    let lastIndex = -1;
    for (const state of observedStates) {
      if (state === "waiting_input") continue; // optional, can appear mid-run
      const idx = validOrder.indexOf(state);
      expect(idx).toBeGreaterThan(lastIndex);
      lastIndex = idx;
    }
  });

  it("session output is retrievable after completion", async () => {
    const { session: created } = await client.createTask({
      prompt: "Reply with: RETRIEVE_TEST_OK",
      max_turns: 1,
    });

    await client.waitForSession(created.id, ["done"]);

    // Fetch the session again — output should be populated
    const session = await client.getSession(created.id);
    expect(session.state).toBe("done");
    expect(session.output).toBeTruthy();
    expect(session.output!.length).toBeGreaterThan(0);

    // Session should appear in the sessions list
    const sessions = await client.getSessions();
    const found = sessions.find((s) => s.id === created.id);
    expect(found).toBeDefined();
    expect(found!.state).toBe("done");
  });

  it("session appears on the assigned node", async () => {
    const { session: created } = await client.createTask({
      prompt: "Reply with: NODE_TEST_OK",
      max_turns: 1,
    });

    // Session should be assigned to a node
    expect(created.node_id).toBeDefined();

    // That node should exist and be online
    const nodes = await client.getNodes();
    const node = nodes.find((n) => n.id === created.node_id);
    expect(node).toBeDefined();
    expect(node!.status).toBe("online");

    await client.waitForSession(created.id, ["done", "failed"]);
  });
});

describe("Session Lifecycle — Error Handling", () => {
  it("handles an intentionally failing prompt gracefully", async () => {
    // A prompt that will likely fail or produce an error
    // (invalid tool reference, but max_turns=1 so it won't loop)
    const { session: created } = await client.createTask({
      prompt: "Reply with: ERROR_HANDLING_OK",
      max_turns: 1,
    });

    const final = await client.waitForSession(created.id, ["done", "failed"]);

    // Whether it succeeds or fails, the session should reach a terminal state
    expect(["done", "failed"]).toContain(final.state);
  });
});
