// Authoring helpers for the guided "Create Plan" surface (spec #542).
//
// The create page (`CreateSpecPlanPage`) assembles a `SpecPlan`
// (`{ specs, deps }`, see `spec-plan.ts`) from form rows, then validates
// it through the `compile_dry_run` MCP tool before committing via
// `submit_spec_plan`. These pure helpers turn the in-progress draft into
// the plan body the tools consume and turn the compiler's structured
// error payload into one human line — kept out of the `.tsx` so they can
// be unit-tested and so the page stays inside the file budget.

import type { SpecPlan, SpecRef, SpecDep } from "@/lib/spec-plan"

/** A spec row in the create form. `id` / `kind` may be blank while editing. */
export interface DraftSpec {
  id: string
  kind: string
}

/** Shape of `compile_dry_run`'s structured result (see
 *  `crates/onsager-portal/src/mcp/tools/substrate_specs.rs`). */
export type CompileResult =
  | { ok: true; execution_plan: { node_count: number; edge_count: number } }
  | { ok: false; error: CompileError }

export interface CompileError {
  kind: string
  message: string
  spec_id?: string
  spec_kind?: string
  violations?: { message: string }[]
}

/** A spec row is "complete" once both id and kind are non-blank. */
export function isComplete(spec: DraftSpec): boolean {
  return spec.id.trim().length > 0 && spec.kind.trim().length > 0
}

/**
 * Build the `SpecPlan` body from the draft. Only complete spec rows are
 * included; deps are kept only when both endpoints reference an included
 * spec id (a dep dangling off a half-typed row would otherwise make every
 * keystroke a compile error). Trims ids so trailing whitespace doesn't
 * fork identity between the node and its edges.
 */
export function draftToPlan(specs: DraftSpec[], deps: SpecDep[]): SpecPlan {
  const complete = specs.filter(isComplete)
  const ids = new Set(complete.map((s) => s.id.trim()))
  const planSpecs: SpecRef[] = complete.map((s) => ({
    id: s.id.trim(),
    kind: s.kind.trim(),
  }))
  const planDeps = deps.filter(
    (d) => d.from && d.to && ids.has(d.from) && ids.has(d.to),
  )
  return { specs: planSpecs, deps: planDeps }
}

/** De-duplicate specs by id (last wins) so a transient duplicate-id draft
 *  doesn't hand React Flow colliding node keys while the compiler flags it. */
export function dedupeById(plan: SpecPlan): SpecPlan {
  const byId = new Map<string, SpecRef>()
  for (const s of plan.specs) byId.set(s.id, s)
  return { specs: [...byId.values()], deps: plan.deps }
}

/** Parse a `compile_dry_run` result envelope; `null` if it isn't JSON. */
export function parseCompileResult(text: string | undefined): CompileResult | null {
  if (!text) return null
  try {
    return JSON.parse(text) as CompileResult
  } catch {
    return null
  }
}

/** One human-readable line for a compile failure, suitable for an inline
 *  error banner. Invariant batches list each violation. */
export function compileErrorMessage(err: CompileError): string {
  if (err.kind === "invariant" && err.violations?.length) {
    return err.violations.map((v) => v.message).join("; ")
  }
  return err.message || `Compile failed (${err.kind}).`
}
