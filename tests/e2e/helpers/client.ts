/**
 * Onsager E2E test client.
 *
 * Talks to a running Onsager instance (Stiglab API) via HTTP + SSE.
 * Configure target via ONSAGER_URL env var (default: http://localhost:3000).
 */

// ── Types ──────────────────────────────────────────────────────────

export interface Node {
  id: string;
  name: string;
  hostname: string;
  status: "online" | "offline" | "draining";
  max_sessions: number;
  active_sessions: number;
  last_heartbeat: string;
  registered_at: string;
}

export interface Session {
  id: string;
  task_id: string;
  node_id: string;
  state: SessionState;
  prompt: string;
  output: string | null;
  working_dir: string | null;
  created_at: string;
  updated_at: string;
}

export type SessionState =
  | "pending"
  | "dispatched"
  | "running"
  | "waiting_input"
  | "done"
  | "failed";

export interface TaskRequest {
  prompt: string;
  node_id?: string;
  working_dir?: string;
  allowed_tools?: string[];
  max_turns?: number;
  model?: string;
  system_prompt?: string;
  permission_mode?: string;
}

export interface SpineEvent {
  id: number;
  stream_id: string;
  stream_type: string;
  event_type: string;
  data: Record<string, unknown>;
  actor: string;
  created_at: string;
}

export interface LogChunk {
  chunk: string;
  stream: "stdout" | "stderr";
}

export interface LogEvent {
  state: string;
  chunks: LogChunk[];
}

export interface HealthResponse {
  status: string;
  version?: string;
}

// ── Errors ─────────────────────────────────────────────────────────

export class ApiError extends Error {
  constructor(
    message: string,
    public status: number,
    public body?: unknown,
  ) {
    super(`${message} (HTTP ${status})`);
    this.name = "ApiError";
  }
}

export class TimeoutError extends Error {
  constructor(
    message: string,
    public lastState?: string,
  ) {
    super(message);
    this.name = "TimeoutError";
  }
}

// ── Client ─────────────────────────────────────────────────────────

export interface OnsagerClient {
  readonly baseUrl: string;

  /** GET /api/health */
  health(): Promise<HealthResponse>;

  /** GET /api/nodes */
  getNodes(): Promise<Node[]>;

  /** GET /api/sessions */
  getSessions(): Promise<Session[]>;

  /** GET /api/sessions/:id */
  getSession(id: string): Promise<Session>;

  /** POST /api/tasks */
  createTask(req: TaskRequest): Promise<{ task: unknown; session: Session }>;

  /** GET /api/spine/events */
  getSpineEvents(params?: {
    stream_type?: string;
    event_type?: string;
    limit?: number;
  }): Promise<SpineEvent[]>;

  /**
   * Poll GET /api/sessions/:id until state matches one of `targetStates`.
   * Throws TimeoutError if not reached within `timeoutMs`.
   */
  waitForSession(
    id: string,
    targetStates: SessionState[],
    timeoutMs?: number,
  ): Promise<Session>;

  /**
   * Stream SSE logs from GET /api/sessions/:id/logs.
   * Yields LogEvent objects as they arrive. Terminates when the
   * server closes the stream (session reaches terminal state).
   */
  streamLogs(
    sessionId: string,
    signal?: AbortSignal,
  ): AsyncGenerator<LogEvent, void, unknown>;
}

// ── Implementation ─────────────────────────────────────────────────

export function createClient(baseUrl?: string): OnsagerClient {
  const url = (
    baseUrl ?? process.env.ONSAGER_URL ?? "http://localhost:3000"
  ).replace(/\/$/, "");

  async function request<T>(
    path: string,
    options?: RequestInit,
  ): Promise<T> {
    const res = await fetch(`${url}${path}`, {
      ...options,
      headers: {
        "Content-Type": "application/json",
        ...options?.headers,
      },
    });

    if (!res.ok) {
      const body = await res.json().catch(() => ({ error: res.statusText }));
      throw new ApiError(
        body.error ?? res.statusText,
        res.status,
        body,
      );
    }

    return res.json() as Promise<T>;
  }

  const client: OnsagerClient = {
    baseUrl: url,

    health() {
      return request<HealthResponse>("/api/health");
    },

    async getNodes() {
      const { nodes } = await request<{ nodes: Node[] }>("/api/nodes");
      return nodes;
    },

    async getSessions() {
      const { sessions } = await request<{ sessions: Session[] }>(
        "/api/sessions",
      );
      return sessions;
    },

    async getSession(id: string) {
      const { session } = await request<{ session: Session }>(
        `/api/sessions/${id}`,
      );
      return session;
    },

    createTask(req: TaskRequest) {
      return request<{ task: unknown; session: Session }>("/api/tasks", {
        method: "POST",
        body: JSON.stringify(req),
      });
    },

    async getSpineEvents(params) {
      const qs = params
        ? "?" +
          new URLSearchParams(
            Object.entries(params)
              .filter(([, v]) => v != null)
              .map(([k, v]) => [k, String(v)]),
          ).toString()
        : "";
      const { events } = await request<{ events: SpineEvent[] }>(
        `/api/spine/events${qs}`,
      );
      return events;
    },

    async waitForSession(
      id: string,
      targetStates: SessionState[],
      timeoutMs = 180_000,
    ): Promise<Session> {
      const deadline = Date.now() + timeoutMs;
      const pollInterval = 2_000;

      while (Date.now() < deadline) {
        const session = await client.getSession(id);

        if (targetStates.includes(session.state)) {
          return session;
        }

        // Fail fast if session hit a terminal state we didn't expect
        if (
          (session.state === "failed" || session.state === "done") &&
          !targetStates.includes(session.state)
        ) {
          throw new Error(
            `Session ${id} reached unexpected terminal state "${session.state}" ` +
            `(expected one of: ${targetStates.join(", ")}). ` +
            `Output: ${session.output?.slice(0, 500) ?? "(none)"}`,
          );
        }

        await sleep(pollInterval);
      }

      const last = await client.getSession(id);
      throw new TimeoutError(
        `Session ${id} did not reach state [${targetStates.join(", ")}] ` +
        `within ${timeoutMs}ms (last state: ${last.state})`,
        last.state,
      );
    },

    async *streamLogs(
      sessionId: string,
      signal?: AbortSignal,
    ): AsyncGenerator<LogEvent, void, unknown> {
      const res = await fetch(`${url}/api/sessions/${sessionId}/logs`, {
        headers: { Accept: "text/event-stream" },
        signal,
      });

      if (!res.ok) {
        throw new ApiError(
          `Failed to open log stream for session ${sessionId}`,
          res.status,
        );
      }

      const reader = res.body!.getReader();
      const decoder = new TextDecoder();
      let buffer = "";

      try {
        while (true) {
          const { done, value } = await reader.read();
          if (done) break;

          buffer += decoder.decode(value, { stream: true });

          // SSE events are separated by double newlines
          const parts = buffer.split("\n\n");
          buffer = parts.pop()!;

          for (const part of parts) {
            for (const line of part.split("\n")) {
              if (line.startsWith("data: ")) {
                try {
                  const event: LogEvent = JSON.parse(line.slice(6));
                  yield event;
                } catch {
                  // skip malformed data lines
                }
              }
            }
          }
        }

        // Process any remaining buffer
        if (buffer.trim()) {
          for (const line of buffer.split("\n")) {
            if (line.startsWith("data: ")) {
              try {
                const event: LogEvent = JSON.parse(line.slice(6));
                yield event;
              } catch {
                // skip
              }
            }
          }
        }
      } finally {
        reader.releaseLock();
      }
    },
  };

  return client;
}

// ── Utilities ──────────────────────────────────────────────────────

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

/**
 * Collect all log chunks from an SSE stream into a single string.
 * Returns when the stream closes.
 */
export async function collectLogs(
  client: OnsagerClient,
  sessionId: string,
  signal?: AbortSignal,
): Promise<{ output: string; states: string[] }> {
  let output = "";
  const states: string[] = [];

  for await (const event of client.streamLogs(sessionId, signal)) {
    if (event.state && !states.includes(event.state)) {
      states.push(event.state);
    }
    for (const chunk of event.chunks) {
      output += chunk.chunk;
    }
  }

  return { output, states };
}

/**
 * Pre-flight check: verify the Onsager instance is reachable and
 * has at least one online node with capacity.
 */
export async function preflight(client: OnsagerClient): Promise<void> {
  // 1. Health check
  const health = await client.health();
  if (health.status !== "ok") {
    throw new Error(
      `Onsager health check failed: ${JSON.stringify(health)}`,
    );
  }

  // 2. At least one online node
  const nodes = await client.getNodes();
  const onlineNodes = nodes.filter((n) => n.status === "online");
  if (onlineNodes.length === 0) {
    throw new Error(
      "No online nodes found. Is the built-in runner or an external agent connected?\n" +
      `All nodes: ${JSON.stringify(nodes.map((n) => ({ name: n.name, status: n.status })))}`,
    );
  }

  // 3. At least one node has capacity
  const available = onlineNodes.filter(
    (n) => n.active_sessions < n.max_sessions,
  );
  if (available.length === 0) {
    throw new Error(
      "All online nodes are at capacity. Wait for sessions to complete or increase max_sessions.\n" +
      `Online nodes: ${JSON.stringify(onlineNodes.map((n) => ({ name: n.name, active: n.active_sessions, max: n.max_sessions })))}`,
    );
  }
}
