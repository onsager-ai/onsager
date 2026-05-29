import { useMemo } from "react"
import { useQuery } from "@tanstack/react-query"
import { api } from "@/lib/api"
import { mcpCallTool } from "@/lib/mcp-client"
import { SpecPlanDAG } from "@/components/chat/SpecPlanDAG"
import { runStateFromEvents, type SpecPlan } from "@/lib/spec-plan"

// F2 (#504, ADR 0023): the run-time counterpart of F1's authored graph.
// Renders a launched Spec Plan with live per-spec state overlaid on the
// same DAG. Sources progress from the substrate `node.*` events the
// scheduler emits (same-origin via portal's `/api/spine/events`), maps
// each node back to its spec via `get_execution_plan`'s `node_index`,
// and keys the run by the derived `plan_id` `run_spec_plan` returned.

interface SpecPlanRunViewProps {
  workspaceId: string
  specPlanId: string
  /** Derived run id from `run_spec_plan`'s result. */
  planId: string
}

/** Parse the first text block of an MCP tool result as JSON. */
function parseResult(text: string | undefined): unknown {
  if (!text) return null
  try {
    return JSON.parse(text)
  } catch {
    return null
  }
}

export function SpecPlanRunView({
  workspaceId,
  specPlanId,
  planId,
}: SpecPlanRunViewProps) {
  // The authored plan body — fetched once.
  const planQuery = useQuery({
    queryKey: ["spec-plan", workspaceId, specPlanId],
    queryFn: async () => {
      const res = await mcpCallTool("get_spec_plan", {
        workspace_id: workspaceId,
        spec_plan_id: specPlanId,
      })
      const parsed = parseResult(res.content?.[0]?.text) as
        | { spec_plan?: { plan?: SpecPlan } }
        | null
      return parsed?.spec_plan?.plan ?? null
    },
  })

  // node_id → spec_id, so node events attribute to the right spec.
  const nodeIndexQuery = useQuery({
    queryKey: ["spec-plan-node-index", workspaceId, specPlanId],
    queryFn: async () => {
      const res = await mcpCallTool("get_execution_plan", {
        workspace_id: workspaceId,
        spec_plan_id: specPlanId,
      })
      const parsed = parseResult(res.content?.[0]?.text) as
        | { execution_plan?: { node_index?: Record<string, string> } }
        | null
      return parsed?.execution_plan?.node_index ?? {}
    },
  })

  // Live substrate node events, polled. Filtered client-side to this
  // plan via `data.plan_id` (the stream-type filter narrows the read;
  // the per-plan filter happens in `runStateFromEvents`).
  const eventsQuery = useQuery({
    queryKey: ["spec-plan-run-events", workspaceId, planId],
    queryFn: () =>
      api.getSpineEvents(workspaceId, { stream_type: "substrate", limit: 200 }),
    refetchInterval: 3000,
  })

  const runState = useMemo(() => {
    const events = eventsQuery.data?.events ?? []
    const nodeIndex = nodeIndexQuery.data ?? {}
    return runStateFromEvents(events, planId, nodeIndex)
  }, [eventsQuery.data, nodeIndexQuery.data, planId])

  const plan = planQuery.data
  if (!plan) {
    return (
      <p className="text-xs text-muted-foreground">
        {planQuery.isLoading ? "Loading plan…" : "Plan graph unavailable."}
      </p>
    )
  }

  return <SpecPlanDAG plan={plan} runState={runState} />
}
