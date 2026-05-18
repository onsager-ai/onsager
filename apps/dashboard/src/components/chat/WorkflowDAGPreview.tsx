import { useEffect, useMemo, useRef, useState, type ReactNode } from "react"
import {
  ReactFlow,
  ReactFlowProvider,
  Background,
  Controls,
  type Node,
  type Edge,
} from "@xyflow/react"
import "@xyflow/react/dist/style.css"
import { AlertTriangle } from "lucide-react"
import type { WorkflowDocument } from "@/components/factory/workflows/workflow-draft"
import { Button } from "@/components/ui/button"
import { Textarea } from "@/components/ui/textarea"
import {
  WorkflowYamlError,
  workflowDocumentFromYaml,
  workflowDocumentToYaml,
} from "@/lib/workflow-yaml"

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

function buildGraph(doc: WorkflowDocument): { nodes: Node[]; edges: Edge[] } {
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
            doc.trigger.repo_owner && doc.trigger.repo_name
              ? `${doc.trigger.repo_owner}/${doc.trigger.repo_name}`
              : "GitHub label"
          }
          color="#6366f1"
        />
      ),
    },
    style: nodeStyle("#6366f1"),
  })

  // Stage nodes
  doc.stages.forEach((stage, i) => {
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

    const sourceId = i === 0 ? "trigger" : doc.stages[i - 1].id || `stage-${i - 1}`
    edges.push({
      id: `e-${sourceId}-${id}`,
      source: sourceId,
      target: id,
      animated: true,
      style: { stroke: "#94a3b8", strokeWidth: 1.5 },
    })
  })

  // End node
  const endX = (NODE_W + GAP_X) * (doc.stages.length + 1)
  nodes.push({
    id: "end",
    type: "default",
    position: { x: endX, y: 0 },
    data: { label: <NodeBox title="Done" sub="" color="#10b981" /> },
    style: nodeStyle("#10b981"),
  })

  const lastId =
    doc.stages.length > 0
      ? doc.stages[doc.stages.length - 1].id || `stage-${doc.stages.length - 1}`
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

type ViewMode = "dag" | "yaml"

interface WorkflowDAGPreviewProps {
  draft: WorkflowDocument | null
  /**
   * Called when the YAML view is edited and successfully parses. Optional —
   * when unset, YAML is render-only (paste-edits silently no-op).
   */
  onChange?: (next: WorkflowDocument) => void
  /** Optional header status — e.g. "Draft · Saved locally" or "Bound · …". */
  status?: string
  /**
   * Optional action rendered in the header bar next to the view toggle —
   * used by ChatPage to mount the "Bind to a repo →" button (spec #402)
   * without splitting the header surface across components.
   */
  headerAction?: ReactNode
}

function DAGInner({ draft }: { draft: WorkflowDocument | null }) {
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

function YamlInner({
  draft,
  onChange,
}: {
  draft: WorkflowDocument | null
  onChange?: (next: WorkflowDocument) => void
}) {
  // Local text buffer so the user can type freely; we only push parsed
  // changes to the parent on each successful parse. The buffer is keyed
  // by the canonical serialization — when the source draft changes from
  // outside (DAG edit, template pick) the canonical changes, and we
  // either fold that into the buffer (user hasn't edited since the last
  // sync) or leave the in-flight edit alone (user is mid-typing).
  const canonical = useMemo(
    () => (draft ? workflowDocumentToYaml(draft) : ""),
    [draft],
  )
  // `override` is the in-flight text buffer the user is typing. While it
  // matches the last-synced canonical, an external draft update folds
  // straight into the textarea (template pick, DAG edit). Once the user
  // diverges, the override sticks until they re-sync (clear text, etc.).
  const [override, setOverride] = useState<string | null>(null)
  const [parseError, setParseError] = useState<string | null>(null)
  const lastSyncedCanonicalRef = useRef(canonical)

  // Sync derived text state when the canonical changes from outside.
  // This is the classic "synchronize state with an external source"
  // useEffect pattern carved out by React's docs; the lint rule's
  // generic warning doesn't apply.
  useEffect(() => {
    if (canonical === lastSyncedCanonicalRef.current) return
    const prevSynced = lastSyncedCanonicalRef.current
    lastSyncedCanonicalRef.current = canonical
    // Only fold into the textarea when the user hasn't typed since our
    // last sync — otherwise we'd clobber an in-flight edit.
    if (override === null || override === prevSynced) {
      // eslint-disable-next-line react-hooks/set-state-in-effect
      setOverride(null)
      setParseError(null)
    }
  }, [canonical, override])

  const text = override ?? canonical

  if (!draft) {
    return (
      <div className="flex h-full flex-col items-center justify-center gap-3 text-center">
        <div className="text-4xl opacity-20">{ "{ }" }</div>
        <p className="text-sm font-medium text-muted-foreground">
          Workflow YAML
        </p>
        <p className="max-w-xs text-xs text-muted-foreground/70">
          As you design a workflow, the YAML configuration will appear here.
        </p>
      </div>
    )
  }

  return (
    <div className="flex h-full flex-col">
      <Textarea
        value={text}
        onChange={(e) => {
          const next = e.target.value
          setOverride(next)
          if (!onChange) return
          try {
            const parsed = workflowDocumentFromYaml(next)
            setParseError(null)
            onChange(parsed)
          } catch (err) {
            setParseError(
              err instanceof WorkflowYamlError ? err.message : String(err),
            )
          }
        }}
        spellCheck={false}
        className="flex-1 resize-none rounded-none border-0 bg-transparent font-mono text-xs"
        aria-label="Workflow configuration as YAML"
      />
      {parseError ? (
        <div
          role="alert"
          className="flex items-start gap-2 border-t border-destructive/40 bg-destructive/5 px-3 py-2 text-xs text-destructive"
        >
          <AlertTriangle className="mt-0.5 h-3.5 w-3.5 shrink-0" />
          <span>Couldn&apos;t parse: {parseError}</span>
        </div>
      ) : null}
    </div>
  )
}

export function WorkflowDAGPreview({
  draft,
  onChange,
  status,
  headerAction,
}: WorkflowDAGPreviewProps) {
  const [view, setView] = useState<ViewMode>("dag")

  return (
    <ReactFlowProvider>
      <div className="flex h-full flex-col overflow-hidden border-l">
        <div className="flex h-10 shrink-0 items-center gap-2 border-b px-4">
          <span className="text-xs font-medium text-muted-foreground">
            {status ?? "Workflow preview"}
          </span>
          <div className="ml-auto flex items-center gap-2">
            {headerAction}
            {draft && (
              <span className="mr-1 text-xs text-muted-foreground">
                {draft.stages.length} stage{draft.stages.length !== 1 ? "s" : ""}
              </span>
            )}
            <div
              role="tablist"
              aria-label="Preview mode"
              className="inline-flex rounded-full border bg-muted/40 p-0.5 text-xs"
            >
              <Button
                role="tab"
                aria-selected={view === "dag"}
                aria-label="Diagram view"
                type="button"
                size="sm"
                variant={view === "dag" ? "default" : "ghost"}
                className="h-6 rounded-full px-2.5 text-xs"
                onClick={() => setView("dag")}
              >
                DAG
              </Button>
              <Button
                role="tab"
                aria-selected={view === "yaml"}
                aria-label="Configuration view"
                type="button"
                size="sm"
                variant={view === "yaml" ? "default" : "ghost"}
                className="h-6 rounded-full px-2.5 text-xs"
                onClick={() => setView("yaml")}
              >
                YAML
              </Button>
            </div>
          </div>
        </div>
        <div className="min-h-0 flex-1">
          {view === "dag" ? (
            <DAGInner draft={draft} />
          ) : (
            <YamlInner draft={draft} onChange={onChange} />
          )}
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
