import { useEffect, useMemo, useState } from "react"
import { useNavigate } from "react-router-dom"
import { useQuery } from "@tanstack/react-query"
import {
  AlertTriangle,
  ArrowRight,
  CheckCircle2,
  Loader2,
  Plus,
  Rocket,
  Trash2,
} from "lucide-react"
import { Button } from "@/components/ui/button"
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card"
import { Input } from "@/components/ui/input"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { HitlActionButton } from "@/components/chat/HitlActionButton"
import { SpecPlanDAG } from "@/components/chat/SpecPlanDAG"
import { SpecKindCombobox } from "@/components/specplans/SpecKindCombobox"
import { usePageHeader } from "@/components/layout/PageHeader"
import { mcpCallTool } from "@/lib/mcp-client"
import {
  type DraftSpec,
  compileErrorMessage,
  dedupeById,
  draftToPlan,
  isComplete,
  parseCompileResult,
} from "@/lib/spec-plan-draft"
import type { SpecDep } from "@/lib/spec-plan"
import { useActiveWorkspace } from "@/lib/workspace"

// Guided "Create Plan" surface (spec #542). The authoring third of the
// Spec Plans surface (#514 built list + Run): assemble a `SpecPlan`
// (`{ specs, deps }`) from form rows — never raw JSON — validate it live
// through `compile_dry_run`, preview the resulting DAG, then commit
// through `submit_spec_plan` routed via a HitlCard. No new business
// logic and no new REST endpoint: the dashboard is already an MCP client
// (seam rule), and chat stays the conversational authoring path — this
// is the deterministic path for a known plan (ADR 0025, complement).

function useDebounced<T>(value: T, ms: number): T {
  const [debounced, setDebounced] = useState(value)
  useEffect(() => {
    const t = setTimeout(() => setDebounced(value), ms)
    return () => clearTimeout(t)
  }, [value, ms])
  return debounced
}

export function CreateSpecPlanPage() {
  const workspace = useActiveWorkspace()
  const navigate = useNavigate()
  const plansPath = `/workspaces/${workspace.slug}/spec-plans`
  usePageHeader({ title: "Create Plan", backTo: plansPath })

  const [planId, setPlanId] = useState("")
  const [specs, setSpecs] = useState<DraftSpec[]>([{ id: "", kind: "" }])
  const [deps, setDeps] = useState<SpecDep[]>([])

  const plan = useMemo(() => draftToPlan(specs, deps), [specs, deps])
  const completeSpecs = specs.filter(isComplete)
  const completeIds = completeSpecs.map((s) => s.id.trim())

  // Live lint: recompile the candidate plan (debounced) whenever it has
  // at least one complete spec. The compiler is the source of truth for
  // unknown kinds / cycles / dangling edges — we don't re-implement it.
  const planKey = useDebounced(JSON.stringify(plan), 400)
  const compileQuery = useQuery({
    queryKey: ["compile-dry-run", workspace.id, planKey],
    queryFn: async () => {
      const res = await mcpCallTool("compile_dry_run", {
        workspace_id: workspace.id,
        spec_plan: JSON.parse(planKey),
      })
      if (res.isError) {
        throw new Error(res.content[0]?.text ?? "compile_dry_run failed")
      }
      return parseCompileResult(res.content[0]?.text)
    },
    enabled: plan.specs.length > 0,
  })

  const compileResult = plan.specs.length > 0 ? compileQuery.data : null
  const compiling = plan.specs.length > 0 && compileQuery.isFetching
  const canSubmit =
    planId.trim().length > 0 &&
    completeSpecs.length > 0 &&
    !compiling &&
    compileResult?.ok === true

  const setSpec = (i: number, patch: Partial<DraftSpec>) =>
    setSpecs((prev) => prev.map((s, j) => (j === i ? { ...s, ...patch } : s)))
  const addSpec = () => setSpecs((prev) => [...prev, { id: "", kind: "" }])
  const removeSpec = (i: number) => {
    const removedId = specs[i]?.id.trim()
    setSpecs((prev) => prev.filter((_, j) => j !== i))
    if (removedId) {
      setDeps((prev) =>
        prev.filter((d) => d.from !== removedId && d.to !== removedId),
      )
    }
  }
  const addDep = () => setDeps((prev) => [...prev, { from: "", to: "" }])
  const setDep = (i: number, patch: Partial<SpecDep>) =>
    setDeps((prev) => prev.map((d, j) => (j === i ? { ...d, ...patch } : d)))
  const removeDep = (i: number) =>
    setDeps((prev) => prev.filter((_, j) => j !== i))

  return (
    <div className="mx-auto max-w-2xl space-y-6">
      <div className="hidden space-y-1 md:block">
        <h1 className="text-2xl font-bold tracking-tight">Create Plan</h1>
        <p className="text-sm text-muted-foreground">
          Add specs and their dependencies, preview the execution graph,
          then submit. The plan runs each spec in dependency order.
        </p>
      </div>

      <Card>
        <CardHeader className="pb-3">
          <CardTitle className="text-base">Plan</CardTitle>
        </CardHeader>
        <CardContent className="space-y-2">
          <label htmlFor="plan-id" className="text-sm font-medium">
            Plan ID
          </label>
          <Input
            id="plan-id"
            value={planId}
            onChange={(e) => setPlanId(e.target.value)}
            placeholder="e.g. github:42 or release-2026-06"
          />
          <p className="text-xs text-muted-foreground">
            A stable identifier for this plan. Reuse a GitHub issue
            reference or any memorable slug.
          </p>
        </CardContent>
      </Card>

      <Card>
        <CardHeader className="pb-3">
          <CardTitle className="text-base">Specs</CardTitle>
        </CardHeader>
        <CardContent className="space-y-3">
          {specs.map((spec, i) => (
            <div key={i} className="flex items-center gap-2">
              <Input
                aria-label={`Spec ${i + 1} id`}
                value={spec.id}
                onChange={(e) => setSpec(i, { id: e.target.value })}
                placeholder="spec id (e.g. github:42)"
                className="flex-1"
              />
              <div className="w-44 shrink-0">
                <SpecKindCombobox
                  workspaceId={workspace.id}
                  value={spec.kind}
                  onChange={(kind) => setSpec(i, { kind })}
                  ariaLabel={`Spec ${i + 1} kind`}
                />
              </div>
              <Button
                type="button"
                variant="ghost"
                size="icon"
                aria-label={`Remove spec ${i + 1}`}
                onClick={() => removeSpec(i)}
                disabled={specs.length === 1}
              >
                <Trash2 className="h-4 w-4" />
              </Button>
            </div>
          ))}
          <Button type="button" variant="outline" size="sm" onClick={addSpec}>
            <Plus className="h-4 w-4" />
            Add spec
          </Button>
        </CardContent>
      </Card>

      <Card>
        <CardHeader className="pb-3">
          <CardTitle className="text-base">Dependencies</CardTitle>
        </CardHeader>
        <CardContent className="space-y-3">
          {deps.length === 0 ? (
            <p className="text-xs text-muted-foreground">
              No dependencies — every spec runs independently. Add one to
              order a spec after another.
            </p>
          ) : (
            deps.map((dep, i) => (
              <div key={i} className="flex items-center gap-2">
                <DepEndpointSelect
                  ariaLabel={`Dependency ${i + 1} from`}
                  value={dep.from}
                  options={completeIds}
                  onChange={(from) => setDep(i, { from })}
                />
                <ArrowRight className="h-4 w-4 shrink-0 text-muted-foreground" />
                <DepEndpointSelect
                  ariaLabel={`Dependency ${i + 1} to`}
                  value={dep.to}
                  options={completeIds}
                  onChange={(to) => setDep(i, { to })}
                />
                <Button
                  type="button"
                  variant="ghost"
                  size="icon"
                  aria-label={`Remove dependency ${i + 1}`}
                  onClick={() => removeDep(i)}
                >
                  <Trash2 className="h-4 w-4" />
                </Button>
              </div>
            ))
          )}
          <Button
            type="button"
            variant="outline"
            size="sm"
            onClick={addDep}
            disabled={completeIds.length < 2}
          >
            <Plus className="h-4 w-4" />
            Add dependency
          </Button>
        </CardContent>
      </Card>

      <Card>
        <CardHeader className="pb-3">
          <CardTitle className="text-base">Preview</CardTitle>
        </CardHeader>
        <CardContent className="space-y-3">
          <CompileStatus
            hasSpecs={completeSpecs.length > 0}
            compiling={compiling}
            result={compileResult ?? null}
            error={compileQuery.error as Error | null}
          />
          {completeSpecs.length > 0 ? (
            <div className="rounded-md border">
              <SpecPlanDAG plan={dedupeById(plan)} />
            </div>
          ) : (
            <p className="text-sm text-muted-foreground">
              Add a spec to preview the execution graph.
            </p>
          )}
        </CardContent>
      </Card>

      <div className="flex items-center justify-end gap-3">
        <Button type="button" variant="ghost" onClick={() => navigate(plansPath)}>
          Cancel
        </Button>
        <HitlActionButton
          toolName="submit_spec_plan"
          args={{
            workspace_id: workspace.id,
            spec_plan_id: planId.trim(),
            spec_plan: plan,
          }}
          onCommitted={() => navigate(plansPath)}
          disabled={!canSubmit}
        >
          <Rocket className="h-4 w-4" />
          Submit plan
        </HitlActionButton>
      </div>
    </div>
  )
}

function DepEndpointSelect({
  ariaLabel,
  value,
  options,
  onChange,
}: {
  ariaLabel: string
  value: string
  options: string[]
  onChange: (id: string) => void
}) {
  return (
    <Select
      value={value}
      onValueChange={(v) => {
        if (typeof v === "string") onChange(v)
      }}
      items={options.map((id) => ({ value: id, label: id }))}
    >
      <SelectTrigger aria-label={ariaLabel} className="flex-1">
        <SelectValue placeholder="select spec" />
      </SelectTrigger>
      <SelectContent>
        {options.map((id) => (
          <SelectItem key={id} value={id}>
            {id}
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  )
}

function CompileStatus({
  hasSpecs,
  compiling,
  result,
  error,
}: {
  hasSpecs: boolean
  compiling: boolean
  result: ReturnType<typeof parseCompileResult> | null
  error: Error | null
}) {
  if (!hasSpecs) return null
  if (compiling) {
    return (
      <div className="flex items-center gap-2 text-sm text-muted-foreground">
        <Loader2 className="h-4 w-4 animate-spin" />
        Compiling…
      </div>
    )
  }
  if (error) {
    return (
      <div
        role="alert"
        className="flex items-start gap-2 rounded-md border border-destructive/40 bg-destructive/5 p-2 text-sm text-destructive"
      >
        <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0" />
        <span>Couldn&apos;t compile — {error.message}</span>
      </div>
    )
  }
  if (!result) return null
  if (result.ok) {
    return (
      <div className="flex items-center gap-2 text-sm text-emerald-600 dark:text-emerald-400">
        <CheckCircle2 className="h-4 w-4" />
        Compiles — {result.execution_plan.node_count} node
        {result.execution_plan.node_count === 1 ? "" : "s"},{" "}
        {result.execution_plan.edge_count} edge
        {result.execution_plan.edge_count === 1 ? "" : "s"}.
      </div>
    )
  }
  return (
    <div
      role="alert"
      className="flex items-start gap-2 rounded-md border border-destructive/40 bg-destructive/5 p-2 text-sm text-destructive"
    >
      <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0" />
      <span>{compileErrorMessage(result.error)}</span>
    </div>
  )
}
