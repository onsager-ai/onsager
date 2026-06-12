import { useMemo } from "react"
import {
  ReactFlow,
  ReactFlowProvider,
  Background,
  Controls,
  type Node,
  type Edge,
} from "@xyflow/react"
import "@xyflow/react/dist/style.css"
import type { SpecPlan, SpecRunState } from "@/lib/spec-plan"

// Distinct from `WorkflowDAGPreview` (a single workflow's linear stage
// chain): this renders a *Spec Plan* — nodes = specs, edges = deps —
// as a layered DAG (F1 #503). An optional `runState` map overlays
// per-spec execution state for the plan-run progress surface (F2 #504).

const NODE_W = 180
const NODE_H = 56
const GAP_X = 70
const GAP_Y = 24

// Per-run-state visual token. `pending` is the authored-only default.
const STATE_META: Record<SpecRunState, { color: string; label: string }> = {
  pending: { color: "#64748b", label: "Pending" },
  running: { color: "#3b82f6", label: "Running" },
  done: { color: "#10b981", label: "Done" },
  failed: { color: "#ef4444", label: "Failed" },
}

export interface SpecPlanDAGProps {
  plan: SpecPlan
  /**
   * Optional per-spec run state (keyed by `SpecRef.id`). When present,
   * nodes are coloured by state and finished/failed edges stop
   * animating — the F2 plan-run progress overlay. Omit for the
   * authored (pre-run) graph.
   */
  runState?: Record<string, SpecRunState>
  /** Fixed pixel height for the embedded canvas. Defaults to 240. */
  height?: number
}

/**
 * Assign each spec a layer = longest dependency path from a root, so
 * upstream specs sit left of their dependents. Specs with no deps are
 * roots (layer 0). Cycles can't occur — the compiler rejects them
 * (`SpecPlan::validate`) — but a defensive visited-set keeps the
 * layout terminating regardless.
 */
function layerOf(
  id: string,
  incoming: Map<string, string[]>,
  memo: Map<string, number>,
  stack: Set<string>,
): number {
  const cached = memo.get(id)
  if (cached !== undefined) return cached
  if (stack.has(id)) return 0
  stack.add(id)
  const parents = incoming.get(id) ?? []
  const layer =
    parents.length === 0
      ? 0
      : 1 + Math.max(...parents.map((p) => layerOf(p, incoming, memo, stack)))
  stack.delete(id)
  memo.set(id, layer)
  return layer
}

function buildGraph(
  plan: SpecPlan,
  runState?: Record<string, SpecRunState>,
): { nodes: Node[]; edges: Edge[] } {
  const incoming = new Map<string, string[]>()
  for (const spec of plan.specs) incoming.set(spec.id, [])
  for (const dep of plan.deps) {
    if (!incoming.has(dep.to)) incoming.set(dep.to, [])
    incoming.get(dep.to)!.push(dep.from)
  }

  const memo = new Map<string, number>()
  const layers = new Map<string, number>()
  for (const spec of plan.specs) {
    layers.set(spec.id, layerOf(spec.id, incoming, memo, new Set()))
  }
  // Track how many nodes already sit in each layer for vertical stacking.
  const perLayerCount = new Map<number, number>()

  const nodes: Node[] = plan.specs.map((spec) => {
    const layer = layers.get(spec.id) ?? 0
    const row = perLayerCount.get(layer) ?? 0
    perLayerCount.set(layer, row + 1)
    const state = runState?.[spec.id] ?? "pending"
    const meta = STATE_META[state]
    return {
      id: spec.id,
      type: "default",
      position: {
        x: layer * (NODE_W + GAP_X),
        y: row * (NODE_H + GAP_Y),
      },
      data: {
        label: (
          <SpecBox
            title={spec.id}
            kind={spec.kind}
            color={meta.color}
            stateLabel={runState ? meta.label : undefined}
          />
        ),
      },
      style: nodeStyle(meta.color),
    }
  })

  const edges: Edge[] = plan.deps.map((dep) => {
    const toState = runState?.[dep.to]
    return {
      id: `e-${dep.from}-${dep.to}`,
      source: dep.from,
      target: dep.to,
      // Keep animating only while the downstream spec hasn't finished.
      animated: !runState || toState === "running" || toState === undefined,
      style: { stroke: "#94a3b8", strokeWidth: 1.5 },
    }
  })

  return { nodes, edges }
}

function nodeStyle(color: string): React.CSSProperties {
  return {
    width: NODE_W,
    height: NODE_H,
    borderRadius: 8,
    border: `1.5px solid ${color}55`,
    background: `${color}11`,
    padding: 0,
    display: "flex",
    alignItems: "center",
    justifyContent: "center",
  }
}

function SpecBox({
  title,
  kind,
  color,
  stateLabel,
}: {
  title: string
  kind: string
  color: string
  stateLabel?: string
}) {
  return (
    <div style={{ textAlign: "center", lineHeight: 1.25, padding: "0 6px" }}>
      <div
        style={{
          fontSize: 12,
          fontWeight: 600,
          color,
          overflow: "hidden",
          textOverflow: "ellipsis",
          whiteSpace: "nowrap",
        }}
      >
        {title}
      </div>
      <div style={{ fontSize: 10, color: "#94a3b8", marginTop: 1 }}>
        {kind}
        {stateLabel ? ` · ${stateLabel}` : ""}
      </div>
    </div>
  )
}

/**
 * Render a Spec Plan as a specs+deps DAG. Read-only — nodes are not
 * draggable or connectable; this is a preview, not an editor.
 */
export function SpecPlanDAG({ plan, runState, height = 240 }: SpecPlanDAGProps) {
  const { nodes, edges } = useMemo(
    () => buildGraph(plan, runState),
    [plan, runState],
  )

  if (plan.specs.length === 0) {
    return (
      <div
        className="flex items-center justify-center rounded-md border text-xs text-muted-foreground"
        style={{ height }}
      >
        Empty plan — no specs to show.
      </div>
    )
  }

  return (
    <div className="overflow-hidden rounded-md border" style={{ height }}>
      <ReactFlowProvider>
        <ReactFlow
          nodes={nodes}
          edges={edges}
          fitView
          fitViewOptions={{ padding: 0.2 }}
          nodesDraggable={false}
          nodesConnectable={false}
          elementsSelectable={false}
          proOptions={{ hideAttribution: true }}
        >
          <Background color="#94a3b820" gap={20} size={1} />
          <Controls showInteractive={false} position="bottom-right" />
        </ReactFlow>
      </ReactFlowProvider>
    </div>
  )
}
