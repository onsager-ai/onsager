import { useQuery } from "@tanstack/react-query"
import { api, type Workspace } from "@/lib/api"
import type { ChatScope } from "@/lib/chat/scope"

// Renders the scope path as a breadcrumb in the chat header:
//
//   workspace          → acme-corp
//   workflow:<id>      → acme-corp / Auto-merge on green
//   run:<runId>        → acme-corp / Auto-merge on green / Run #42
//   verdict:<verdictId>→ acme-corp / Auto-merge on green / Run #42 / Verdict
//
// Labels are looked up via React Query against existing dashboard
// endpoints. Cache hits render instantly; misses show the raw id as a
// fallback while the query resolves. We deliberately do NOT fire a
// blocking load — the breadcrumb is decoration, not a gate.

interface ChatBreadcrumbProps {
  workspace: Workspace
  scope: ChatScope
}

export function ChatBreadcrumb({ workspace, scope }: ChatBreadcrumbProps) {
  const segments: string[] = [workspace.slug]

  // Workflow lookup. Fired for `workflow`, `run`, `verdict` scopes —
  // run + verdict scopes need the workflow name in the breadcrumb too,
  // but they don't know its id directly. So for those we resolve via
  // the run first (below) and use the workflow_id off the run.
  if (scope.type === "workflow" && scope.id) {
    return (
      <Crumbs
        segments={[
          ...segments,
          <WorkflowLabel key="wf" workflowId={scope.id} />,
        ]}
      />
    )
  }
  if (scope.type === "run" && scope.id) {
    return <RunCrumbs segments={segments} runId={scope.id} />
  }
  if (scope.type === "verdict" && scope.id) {
    return (
      <Crumbs
        segments={[
          ...segments,
          // Verdict scopes don't carry their parent run id; render
          // the leaf without ascending. Once verdict deep-links
          // include the run id, expand here.
          <span key="vd" className="text-foreground">Verdict</span>,
        ]}
      />
    )
  }
  return <Crumbs segments={segments} />
}

function Crumbs({ segments }: { segments: Array<string | React.ReactNode> }) {
  return (
    <nav aria-label="Chat scope" className="flex items-center gap-1.5 text-xs">
      {segments.map((seg, i) => (
        <span key={i} className="flex items-center gap-1.5 min-w-0">
          {i > 0 ? (
            <span className="text-muted-foreground/60" aria-hidden="true">
              /
            </span>
          ) : null}
          <span className="truncate text-foreground">{seg}</span>
        </span>
      ))}
    </nav>
  )
}

function WorkflowLabel({ workflowId }: { workflowId: string }) {
  const { data } = useQuery({
    queryKey: ["workflow", workflowId],
    queryFn: () => api.getWorkflow(workflowId),
    staleTime: 30_000,
  })
  return <>{data?.workflow?.name ?? `Workflow ${workflowId.slice(0, 6)}`}</>
}

// Run crumbs: one lookup, since `RunDetail` carries the workflow row
// inline. The workflow + run crumbs render once the run resolves.
function RunCrumbs({
  segments,
  runId,
}: {
  segments: Array<string | React.ReactNode>
  runId: string
}) {
  const { data: runData } = useQuery({
    queryKey: ["run", runId],
    queryFn: () => api.getRun(runId),
    staleTime: 30_000,
  })
  const workflowName =
    runData?.workflow?.name ?? `Workflow`
  const runLabel = `Run ${runId.slice(0, 6)}`
  return (
    <Crumbs
      segments={[
        ...segments,
        <span key="wf" className="text-foreground">{workflowName}</span>,
        <span key="run" className="text-foreground">{runLabel}</span>,
      ]}
    />
  )
}
