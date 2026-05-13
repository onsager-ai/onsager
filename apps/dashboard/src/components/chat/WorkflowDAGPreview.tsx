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
import type { WorkflowDraft } from "@/components/factory/workflows/workflow-draft"

// Gate kind → human label + colour token
const GATE_META: Record<string, { label: string; color: string }> = {
  "agent-session": { label: "Agent session", color: "#3b82f6" },
  "external-check": { label: "CI check", color: "#a855f7" },
  governance: { label: "Governance", color: "#f59e0b" },
  "manual-approval": { label: "Manual approval", color: "#10b981" },
}

const NODE_W = 180
const NODE_H = 60
const GAP_X = 60

function buildGraph(draft: WorkflowDraft): { nodes: Node[]; edges: Edge[] } {
  const nodes: Node[] = []
  const edges: Edge[] = []

  // Trigger node
  nodes.push({
    id: "trigger",
    type: "default",
    position: { x: 0, y: 0 },
    data: {
      label: (
        <NodeBox
          title="Trigger"
          sub={
            draft.trigger.repo_owner && draft.trigger.repo_name
              ? `${draft.trigger.repo_owner}/${draft.trigger.repo_name}`
              : "GitHub label"
          }
          color="#6366f1"
        />
      ),
    },
    style: nodeStyle("#6366f1"),
  })

  // Stage nodes
  draft.stages.forEach((stage, i) => {
    const id = stage.id || `stage-${i}`
    const meta = GATE_META[stage.gate_kind] ?? {
      label: stage.gate_kind,
      color: "#64748b",
    }
    nodes.push({
      id,
      type: "default",
      position: { x: (NODE_W + GAP_X) * (i + 1), y: 0 },
      data: {
        label: (
          <NodeBox
            title={stage.name || meta.label}
            sub={meta.label}
            color={meta.color}
          />
        ),
      },
      style: nodeStyle(meta.color),
    })

    const sourceId = i === 0 ? "trigger" : draft.stages[i - 1].id || `stage-${i - 1}`
    edges.push({
      id: `e-${sourceId}-${id}`,
      source: sourceId,
      target: id,
      animated: true,
      style: { stroke: "#94a3b8", strokeWidth: 1.5 },
    })
  })

  // End node
  const endX = (NODE_W + GAP_X) * (draft.stages.length + 1)
  nodes.push({
    id: "end",
    type: "default",
    position: { x: endX, y: 0 },
    data: { label: <NodeBox title="Done" sub="" color="#10b981" /> },
    style: nodeStyle("#10b981"),
  })

  const lastId =
    draft.stages.length > 0
      ? draft.stages[draft.stages.length - 1].id || `stage-${draft.stages.length - 1}`
      : "trigger"
  edges.push({
    id: `e-${lastId}-end`,
    source: lastId,
    target: "end",
    animated: true,
    style: { stroke: "#94a3b8", strokeWidth: 1.5 },
  })

  return { nodes, edges }
}

function nodeStyle(color: string): React.CSSProperties {
  return {
    width: NODE_W,
    height: NODE_H,
    borderRadius: 8,
    border: `1.5px solid ${color}33`,
    background: `${color}11`,
    padding: 0,
    display: "flex",
    alignItems: "center",
    justifyContent: "center",
  }
}

function NodeBox({ title, sub, color }: { title: string; sub: string; color: string }) {
  return (
    <div style={{ textAlign: "center", lineHeight: 1.3 }}>
      <div style={{ fontSize: 12, fontWeight: 600, color }}>{title}</div>
      {sub ? <div style={{ fontSize: 10, color: "#94a3b8", marginTop: 2 }}>{sub}</div> : null}
    </div>
  )
}

interface WorkflowDAGPreviewProps {
  draft: WorkflowDraft | null
}

function DAGInner({ draft }: WorkflowDAGPreviewProps) {
  const { nodes, edges } = useMemo(
    () =>
      draft
        ? buildGraph(draft)
        : { nodes: [], edges: [] },
    [draft],
  )

  if (!draft) {
    return (
      <div className="flex h-full flex-col items-center justify-center gap-3 text-center">
        <div className="text-4xl opacity-20">◈</div>
        <p className="text-sm font-medium text-muted-foreground">
          Workflow preview
        </p>
        <p className="max-w-xs text-xs text-muted-foreground/70">
          As you design a workflow with the agent, the graph will appear here.
        </p>
      </div>
    )
  }

  return (
    <ReactFlow
      nodes={nodes}
      edges={edges}
      fitView
      fitViewOptions={{ padding: 0.3 }}
      nodesDraggable={false}
      nodesConnectable={false}
      elementsSelectable={false}
      proOptions={{ hideAttribution: true }}
    >
      <Background color="#94a3b820" gap={20} size={1} />
      <Controls showInteractive={false} position="bottom-right" />
    </ReactFlow>
  )
}

export function WorkflowDAGPreview({ draft }: WorkflowDAGPreviewProps) {
  return (
    <ReactFlowProvider>
      <div className="flex h-full flex-col overflow-hidden border-l">
        <div className="flex h-10 shrink-0 items-center border-b px-4">
          <span className="text-xs font-medium text-muted-foreground">
            Workflow preview
          </span>
          {draft && (
            <span className="ml-auto text-xs text-muted-foreground">
              {draft.stages.length} stage{draft.stages.length !== 1 ? "s" : ""}
            </span>
          )}
        </div>
        <div className="min-h-0 flex-1">
          <DAGInner draft={draft} />
        </div>
        {draft && (
          <div className="shrink-0 border-t px-4 py-2">
            <p className="truncate text-xs font-medium">{draft.name || "Untitled workflow"}</p>
            {draft.trigger.repo_owner && (
              <p className="text-xs text-muted-foreground">
                {draft.trigger.repo_owner}/{draft.trigger.repo_name} · {draft.trigger.label}
              </p>
            )}
          </div>
        )}
      </div>
    </ReactFlowProvider>
  )
}
