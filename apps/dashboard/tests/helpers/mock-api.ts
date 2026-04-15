import type { Node, Session } from "@/lib/api";

/** Factory for mock Node objects. */
export function mockNode(overrides: Partial<Node> = {}): Node {
  return {
    id: "node-001",
    name: "test-node",
    hostname: "localhost",
    status: "online",
    max_sessions: 4,
    active_sessions: 1,
    last_heartbeat: new Date().toISOString(),
    registered_at: new Date().toISOString(),
    ...overrides,
  };
}

/** Factory for mock Session objects. */
export function mockSession(overrides: Partial<Session> = {}): Session {
  return {
    id: "sess-001",
    task_id: "task-001",
    node_id: "node-001",
    state: "running",
    prompt: "Fix the login bug",
    output: null,
    working_dir: "/app",
    created_at: new Date().toISOString(),
    updated_at: new Date().toISOString(),
    ...overrides,
  };
}

/** Create N mock sessions with varying states. */
export function mockSessions(count: number): Session[] {
  const states: Session["state"][] = [
    "pending",
    "dispatched",
    "running",
    "waiting_input",
    "done",
    "failed",
  ];
  return Array.from({ length: count }, (_, i) =>
    mockSession({
      id: `sess-${String(i).padStart(3, "0")}`,
      task_id: `task-${String(i).padStart(3, "0")}`,
      state: states[i % states.length],
      prompt: `Task ${i}: do something`,
    }),
  );
}

/** Create N mock nodes. */
export function mockNodes(count: number): Node[] {
  const statuses: Node["status"][] = ["online", "offline", "draining"];
  return Array.from({ length: count }, (_, i) =>
    mockNode({
      id: `node-${String(i).padStart(3, "0")}`,
      name: `node-${i}`,
      status: statuses[i % statuses.length],
      active_sessions: i % 4,
    }),
  );
}
