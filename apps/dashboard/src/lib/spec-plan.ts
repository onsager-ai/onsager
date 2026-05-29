// Spec-plan shapes the dashboard renders (F1 #503 / F2 #504, ADR 0023).
//
// Hand-typed for v1, mirroring the Rust `SpecPlan` / `SpecRef` /
// `SpecDep` serde structs in `crates/onsager-substrate/src/spec_plan.rs`.
// The schemars-derived Rust-as-SSOT TS codegen is the same follow-up
// that covers the MCP tool args (see `mcp-tools.ts`); once it lands
// these collapse to generated imports.

import type { SpineEvent } from "@/lib/api/types"

export interface SpecInputs {
  artifacts?: string[]
}

export interface SpecRef {
  id: string
  kind: string
  inputs?: SpecInputs
}

export interface SpecDep {
  from: string
  to: string
}

export interface SpecPlan {
  specs: SpecRef[]
  deps: SpecDep[]
}

/** Per-spec run state overlaid on the plan graph (F2). */
export type SpecRunState = "pending" | "running" | "done" | "failed"

/**
 * Pull a `SpecPlan` out of an MCP tool's arguments. `submit_spec_plan`
 * and `compile_dry_run` carry the full `{ specs, deps }` body under
 * `spec_plan`; other spec-plan tools reference a plan by id and carry
 * no body. Returns `null` when no renderable plan is present.
 */
export function extractSpecPlan(args: Record<string, unknown>): SpecPlan | null {
  const raw = args.spec_plan
  if (!raw || typeof raw !== "object") return null
  const obj = raw as Record<string, unknown>
  const specs = Array.isArray(obj.specs) ? (obj.specs as SpecRef[]) : null
  if (!specs) return null
  const deps = Array.isArray(obj.deps) ? (obj.deps as SpecDep[]) : []
  return { specs, deps }
}

/**
 * Reduce a batch of substrate `node.*` spine events into a per-spec
 * run state, given the `node_id → spec_id` index from
 * `get_execution_plan`. Events are filtered to `planId` (their
 * `data.plan_id`); each node's latest lifecycle event wins, and a
 * spec's state is the max-severity over its nodes
 * (failed > running > done > pending).
 */
export function runStateFromEvents(
  events: SpineEvent[],
  planId: string,
  nodeIndex: Record<string, string>,
): Record<string, SpecRunState> {
  // node_id → latest state seen.
  const nodeState: Record<string, SpecRunState> = {}
  // Events arrive newest-first from the API; walk oldest-first so the
  // last write per node wins.
  for (const e of [...events].reverse()) {
    if (e.data?.plan_id !== planId) continue
    const nodeId = typeof e.data?.node_id === "string" ? e.data.node_id : null
    if (!nodeId) continue
    const next = eventTypeToState(e.event_type)
    if (next) nodeState[nodeId] = next
  }

  const rank: Record<SpecRunState, number> = {
    pending: 0,
    done: 1,
    running: 2,
    failed: 3,
  }
  const out: Record<string, SpecRunState> = {}
  for (const [nodeId, state] of Object.entries(nodeState)) {
    const specId = nodeIndex[nodeId]
    if (!specId) continue
    const prev = out[specId]
    if (prev === undefined || rank[state] > rank[prev]) {
      out[specId] = state
    }
  }
  return out
}

function eventTypeToState(eventType: string): SpecRunState | null {
  switch (eventType) {
    case "node.started":
      return "running"
    case "node.completed":
      return "done"
    case "node.failed":
      return "failed"
    default:
      return null
  }
}
