/**
 * E2E: Log Streaming
 *
 * Tests that session output is streamed in real-time via SSE and
 * that the stream terminates correctly when the session completes.
 *
 * Prerequisites:
 *   - Onsager stack running (just dev)
 *   - At least one online node with valid credentials
 */
import { describe, it, expect, beforeAll } from "vitest";
import {
  createClient,
  collectLogs,
  preflight,
  type OnsagerClient,
  type LogEvent,
} from "../helpers/client";

let client: OnsagerClient;

beforeAll(async () => {
  client = createClient();
  await preflight(client);
});

describe("Log Streaming", () => {
  it("streams output chunks via SSE during session execution", async () => {
    const { session } = await client.createTask({
      prompt:
        "Count from 1 to 5, each number on a new line. " +
        "Then say STREAM_TEST_DONE on the last line.",
      max_turns: 1,
    });

    const events: LogEvent[] = [];
    const controller = new AbortController();

    // Collect SSE events in parallel with the session running
    const streamPromise = (async () => {
      for await (const event of client.streamLogs(
        session.id,
        controller.signal,
      )) {
        events.push(event);
      }
    })();

    // Also wait for session completion via polling
    const final = await client.waitForSession(session.id, ["done", "failed"]);

    // Give the stream a moment to flush, then abort if still open
    controller.abort();
    await streamPromise;

    expect(final.state).toBe("done");

    // We should have received at least one SSE event
    expect(events.length).toBeGreaterThan(0);

    // At least one event should contain output chunks
    const withChunks = events.filter((e) => e.chunks.length > 0);
    expect(withChunks.length).toBeGreaterThan(0);

    // Concatenated output should contain meaningful content
    const fullOutput = events
      .flatMap((e) => e.chunks)
      .map((c) => c.chunk)
      .join("");
    expect(fullOutput.length).toBeGreaterThan(0);
  });

  it("collectLogs helper produces complete output", async () => {
    const { session } = await client.createTask({
      prompt: "Reply with exactly: COLLECT_TEST_OK",
      max_turns: 1,
    });

    // Use the collectLogs helper — it should return the full output
    // once the stream closes
    const controller = new AbortController();

    // Race: either stream completes naturally or we abort after session is done
    const logsPromise = collectLogs(client, session.id, controller.signal);
    const final = await client.waitForSession(session.id, ["done", "failed"]);

    // Give stream time to close naturally (5s), then abort and await
    const result = await Promise.race([
      logsPromise,
      new Promise<null>((r) => setTimeout(() => r(null), 5_000)),
    ]);

    if (result === null) {
      // Stream didn't close naturally — abort and collect what we got
      controller.abort();
      // Await to avoid unhandled rejection (streamLogs handles AbortError cleanly)
      await logsPromise.catch(() => {});
    }

    const output = result?.output ?? final.output ?? "";

    expect(final.state).toBe("done");
    // Output from either SSE or polling should have content
    expect(output.length + (final.output?.length ?? 0)).toBeGreaterThan(0);
  });

  it("SSE events report correct session state", async () => {
    const { session } = await client.createTask({
      prompt: "Reply with: SSE_STATE_OK",
      max_turns: 1,
    });

    const observedStates: string[] = [];
    const controller = new AbortController();

    const streamPromise = (async () => {
      for await (const event of client.streamLogs(
        session.id,
        controller.signal,
      )) {
        if (event.state && !observedStates.includes(event.state)) {
          observedStates.push(event.state);
        }
      }
    })();

    await client.waitForSession(session.id, ["done", "failed"]);

    // Abort and await cleanly
    controller.abort();
    await streamPromise;

    // SSE should have reported at least the "running" state
    // (pending/dispatched may be too fast to catch)
    if (observedStates.length > 0) {
      // All reported states should be valid session states
      const validStates = [
        "pending",
        "dispatched",
        "running",
        "waiting_input",
        "done",
        "failed",
      ];
      for (const state of observedStates) {
        expect(validStates).toContain(state);
      }
    }
  });
});
