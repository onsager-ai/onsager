/**
 * E2E: Multi-Session
 *
 * Tests concurrent session execution and node load distribution.
 * Verifies that the system handles multiple simultaneous sessions
 * correctly without cross-contamination.
 *
 * Prerequisites:
 *   - Onsager stack running (just dev)
 *   - At least one online node with capacity >= 2
 */
import { describe, it, expect, beforeAll } from "vitest";
import {
  createClient,
  preflight,
  type OnsagerClient,
} from "../helpers/client";

let client: OnsagerClient;

beforeAll(async () => {
  client = createClient();
  await preflight(client);

  // Verify enough capacity for concurrent sessions
  const nodes = await client.getNodes();
  const totalCapacity = nodes
    .filter((n) => n.status === "online")
    .reduce((sum, n) => sum + (n.max_sessions - n.active_sessions), 0);

  if (totalCapacity < 2) {
    throw new Error(
      `Need at least 2 available session slots for multi-session tests, ` +
      `but only ${totalCapacity} available. Increase max_sessions or wait for sessions to complete.`,
    );
  }
});

describe("Multi-Session", () => {
  it("runs two sessions concurrently without interference", async () => {
    // Each session has a unique marker to verify no cross-contamination
    const marker1 = `MULTI_A_${Date.now()}`;
    const marker2 = `MULTI_B_${Date.now()}`;

    const [result1, result2] = await Promise.all([
      client.createTask({
        prompt: `Reply with exactly: ${marker1}`,
        max_turns: 1,
      }),
      client.createTask({
        prompt: `Reply with exactly: ${marker2}`,
        max_turns: 1,
      }),
    ]);

    // Both sessions should be created with distinct IDs
    expect(result1.session.id).not.toBe(result2.session.id);

    // Wait for both to complete
    const [final1, final2] = await Promise.all([
      client.waitForSession(result1.session.id, ["done", "failed"]),
      client.waitForSession(result2.session.id, ["done", "failed"]),
    ]);

    expect(final1.state).toBe("done");
    expect(final2.state).toBe("done");

    // Each session should have its own marker in output
    expect(final1.output).toContain(marker1);
    expect(final2.output).toContain(marker2);

    // No cross-contamination
    expect(final1.output).not.toContain(marker2);
    expect(final2.output).not.toContain(marker1);
  });

  it("assigns sessions to available nodes", async () => {
    const { session } = await client.createTask({
      prompt: "Reply with: ASSIGN_TEST_OK",
      max_turns: 1,
    });

    // Should be assigned to an online node
    const nodes = await client.getNodes();
    const assignedNode = nodes.find((n) => n.id === session.node_id);

    expect(assignedNode).toBeDefined();
    expect(assignedNode!.status).toBe("online");

    await client.waitForSession(session.id, ["done", "failed"]);
  });

  it("sessions list shows all created sessions", async () => {
    const marker = `LIST_TEST_${Date.now()}`;

    const { session } = await client.createTask({
      prompt: `Reply with: ${marker}`,
      max_turns: 1,
    });

    // Session should appear in the list immediately
    const sessions = await client.getSessions();
    const found = sessions.find((s) => s.id === session.id);
    expect(found).toBeDefined();
    expect(found!.prompt).toContain(marker);

    await client.waitForSession(session.id, ["done", "failed"]);
  });
});
