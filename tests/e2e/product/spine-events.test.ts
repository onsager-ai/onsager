/**
 * E2E: Spine Events
 *
 * Tests that session lifecycle events are correctly emitted to the
 * event spine and are queryable via the API. This validates the
 * stigmergy feedback loop — other subsystems (forge, ising, synodic)
 * depend on these events for coordination.
 *
 * Prerequisites:
 *   - Onsager stack running with spine (just dev)
 *   - ONSAGER_DATABASE_URL configured for spine integration
 */
import { describe, it, expect, beforeAll } from "vitest";
import {
  createClient,
  preflight,
  type OnsagerClient,
} from "../helpers/client";

let client: OnsagerClient;
let spineAvailable = false;

beforeAll(async () => {
  client = createClient();
  await preflight(client);

  // Check if spine is connected (events endpoint may return empty
  // if ONSAGER_DATABASE_URL is not configured)
  try {
    await client.getSpineEvents({ limit: 1 });
    spineAvailable = true;
  } catch {
    spineAvailable = false;
  }
});

describe("Spine Events", () => {
  it("records session lifecycle events in the spine", async ({ skip }) => {
    if (!spineAvailable) skip();

    const { session } = await client.createTask({
      prompt: "Reply with: SPINE_TEST_OK",
      max_turns: 1,
    });

    const final = await client.waitForSession(session.id, [
      "done",
      "failed",
    ]);
    expect(final.state).toBe("done");

    // Give spine a moment to process the event
    await new Promise((r) => setTimeout(r, 2_000));

    // Query spine for events related to this session
    const events = await client.getSpineEvents({
      stream_type: "stiglab",
      limit: 100,
    });

    // Should have at least one event mentioning our session
    const sessionEvents = events.filter(
      (e) =>
        e.data &&
        ((e.data as Record<string, unknown>).session_id === session.id ||
          String(e.stream_id).includes(session.id)),
    );

    expect(sessionEvents.length).toBeGreaterThan(0);
  });

  it("spine events are queryable by event type", async ({ skip }) => {
    if (!spineAvailable) skip();

    // Query different event types — they should all return valid responses
    const [stiglabEvents, allEvents] = await Promise.all([
      client.getSpineEvents({ stream_type: "stiglab", limit: 10 }),
      client.getSpineEvents({ limit: 10 }),
    ]);

    // Both queries should succeed (even if empty)
    expect(Array.isArray(stiglabEvents)).toBe(true);
    expect(Array.isArray(allEvents)).toBe(true);

    // All stiglab events should have the correct stream_type
    for (const event of stiglabEvents) {
      expect(event.stream_type).toBe("stiglab");
    }

    // Events should have valid structure
    for (const event of allEvents) {
      expect(event.id).toBeDefined();
      expect(event.event_type).toBeDefined();
      expect(event.created_at).toBeDefined();
    }
  });
});
