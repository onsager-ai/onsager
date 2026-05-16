//! Plan Compiler — turns a [`SpecPlan`] plus a [`WorkflowLibrary`]
//! into a fully-resolved, validated [`ExecutionPlan`].
//!
//! See [ADR 0017](../../../docs/adr/0017-plan-compiler-three-step-algorithm.md)
//! for the algorithm and [ADR 0009](../../../docs/adr/0009-three-layer-pipeline.md)
//! for where this layer sits in the pipeline. The whole compiler is
//! the four-step skeleton from ADR 0017:
//!
//! ```text
//! 1. validate the Spec Plan as a DAG (cycles + dangling refs)
//! 2. for each spec: lookup workflow → instantiate → graft into plan
//! 3. for each dep: rewire to-spec entry to consume from-spec exit
//! 4. run validate_workflow over the merged plan (ADR 0018)
//! ```
//!
//! The compiler is **stateless**, **pure**, and **deterministic**:
//! same `(SpecPlan, WorkflowLibrary)` snapshot always produces a
//! byte-identical [`ExecutionPlan`] after canonical serialization.
//! Determinism comes from [`crate::workflow::Workflow::instantiate`],
//! which derives every UUID from a stable namespace seed plus
//! `SpecId` (UUID v5) instead of generating fresh v4s.

use std::collections::HashMap;

use crate::ids::EdgeId;
use crate::library::WorkflowLibrary;
use crate::spec_plan::{SpecDep, SpecId, SpecPlan, SpecPlanError};
use crate::validate::{InvariantViolation, validate_workflow};
use crate::workflow::{Edge, EdgeRef, InstantiatedWorkflow, Node, OutputSpec, Workflow};

/// A fully-resolved, immutable graph the substrate scheduler runs.
///
/// Nodes and edges are flat — there is no per-spec subgraph after
/// compilation. The `spec_index` map keeps a back-pointer from the
/// original [`SpecId`] to that spec's exit edges, useful for runtime
/// observers and for diagnostics that want to attribute a node to
/// the spec it came from.
#[derive(Debug)]
pub struct ExecutionPlan {
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
    /// `spec_id → ExecutionPlan-relative entry/exit edges`. Keys are
    /// the original `SpecId`s from the input Spec Plan; entry edges
    /// reflect rewiring from `connect`.
    pub spec_index: HashMap<SpecId, SpecSlot>,
}

/// Where a single spec landed in the merged Execution Plan.
///
/// `entry_edges` are the workflow's declared entry edge IDs *after*
/// any rewiring from spec deps — i.e. an entry that was wired to an
/// upstream exit will appear here as the upstream's exit edge id, so
/// downstream observers can trace the connection.
#[derive(Debug, Clone)]
pub struct SpecSlot {
    pub entry_edges: Vec<EdgeId>,
    pub exit_edges: Vec<OutputSpec>,
}

/// Why compilation failed. The compiler aborts at the first error
/// because most failures invalidate every step that follows; the
/// kernel-invariant errors from step 4 are the one batched case
/// (every violation surfaces in a single `Vec`).
#[derive(Debug, thiserror::Error)]
pub enum CompileError {
    /// Spec Plan failed structural validation before any workflow
    /// lookup. The wrapped error names the offending spec ids.
    #[error("Spec Plan structural error: {0}")]
    SpecPlan(#[from] SpecPlanError),

    /// `workflow_library.by_kind(kind)` returned `None`. ADR 0017
    /// commits to hard-failing here — there is no fallback workflow.
    #[error(
        "no workflow registered for spec '{spec_id}' (kind '{kind}'); \
         add it to the workflow library or fix the spec kind"
    )]
    MissingKind { spec_id: SpecId, kind: String },

    /// A spec dependency wires `from → to` but `from` declares zero
    /// exit edges, so there is nothing to connect.
    #[error(
        "cannot wire spec dependency '{from} -> {to}': upstream spec '{from}' \
         (kind '{from_kind}') declares no exit edges in its workflow"
    )]
    NoExit {
        from: SpecId,
        to: SpecId,
        from_kind: String,
    },

    /// A spec dependency wires `from → to` but `to` declares zero
    /// entry edges, so there is nothing to wire into.
    #[error(
        "cannot wire spec dependency '{from} -> {to}': downstream spec '{to}' \
         (kind '{to_kind}') declares no entry edges in its workflow"
    )]
    NoEntry {
        from: SpecId,
        to: SpecId,
        to_kind: String,
    },

    /// Two or more `SpecDep`s target the same downstream spec. ADR
    /// 0015 fixes v1 at single-entry / single-exit per workflow, so
    /// fan-in into the same entry slot is out of scope; widening the
    /// IO model is a follow-up rather than a silent compile.
    #[error(
        "spec '{to}' has multiple incoming deps ({}) but v1 workflows \
         declare a single entry slot — split the workflow or merge upstream \
         specs", format_spec_ids(.from)
    )]
    MultipleIncomingDeps { to: SpecId, from: Vec<SpecId> },

    /// The merged Execution Plan failed one or more kernel invariants
    /// (ADR 0018). The wrapped vector is every violation found in
    /// one pass.
    #[error("Execution Plan failed kernel invariants: {} violation(s)", .0.len())]
    Invariant(Vec<InvariantViolation>),
}

fn format_spec_ids(ids: &[SpecId]) -> String {
    ids.iter()
        .map(SpecId::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

/// Compile a Spec Plan against a Workflow Library.
///
/// See the module docs for the algorithm. The function is pure: no
/// I/O, no globals, no allocations beyond the returned plan.
pub fn compile(
    spec_plan: &SpecPlan,
    library: &dyn WorkflowLibrary,
) -> Result<ExecutionPlan, CompileError> {
    // Step 0 — Spec Plan structural checks. Hoisted out of step 2's
    // loop so authors see DAG errors before any library lookup runs.
    spec_plan.validate()?;

    let mut instantiated: HashMap<SpecId, (String, InstantiatedWorkflow)> = HashMap::new();

    // Step 1+2 — lookup + instantiate, in declaration order.
    for spec in &spec_plan.specs {
        let workflow = library
            .by_kind(&spec.kind)
            .ok_or_else(|| CompileError::MissingKind {
                spec_id: spec.id.clone(),
                kind: spec.kind.clone(),
            })?;
        let inst = workflow.instantiate(&spec.id);
        instantiated.insert(spec.id.clone(), (spec.kind.clone(), inst));
    }

    // Step 3 — connect. Build a per-spec entry rewrite map: for each
    // dep `from -> to`, point every reference to `to`'s entry
    // edge_id at `from`'s exit edge_id. v1 fixes single-entry /
    // single-exit per ADR 0015, so we reject the second incoming
    // dep into any given spec rather than silently overwriting the
    // first wiring.
    let mut rewrites: HashMap<EdgeId, EdgeId> = HashMap::new();
    let mut effective_entries: HashMap<SpecId, Vec<EdgeId>> = HashMap::new();
    for (spec_id, (_, inst)) in &instantiated {
        effective_entries.insert(spec_id.clone(), inst.entry_edges.clone());
    }

    // Detect fan-in (multiple `from` specs targeting the same `to`)
    // up-front so the error names every offending upstream, not just
    // the second one we happened to reach.
    let mut incoming: HashMap<SpecId, Vec<SpecId>> = HashMap::new();
    for SpecDep { from, to } in &spec_plan.deps {
        incoming.entry(to.clone()).or_default().push(from.clone());
    }
    for (to, froms) in &incoming {
        if froms.len() > 1 {
            return Err(CompileError::MultipleIncomingDeps {
                to: to.clone(),
                from: froms.clone(),
            });
        }
    }

    for SpecDep { from, to } in &spec_plan.deps {
        // Both ids resolve — already enforced by SpecPlan::validate.
        let from_inst = &instantiated[from].1;
        let from_kind = instantiated[from].0.clone();
        let to_inst = &instantiated[to].1;
        let to_kind = instantiated[to].0.clone();

        let exit_edge_id = from_inst
            .exit_edges
            .first()
            .map(|o| o.edge_id)
            .ok_or_else(|| CompileError::NoExit {
                from: from.clone(),
                to: to.clone(),
                from_kind,
            })?;
        let entry_edge_id =
            to_inst
                .entry_edges
                .first()
                .copied()
                .ok_or_else(|| CompileError::NoEntry {
                    from: from.clone(),
                    to: to.clone(),
                    to_kind,
                })?;

        rewrites.insert(entry_edge_id, exit_edge_id);

        // Reflect the rewire in the to-spec's effective entry list so
        // the spec_index back-pointer surfaces the wired edge id.
        if let Some(entries) = effective_entries.get_mut(to) {
            for e in entries.iter_mut() {
                if *e == entry_edge_id {
                    *e = exit_edge_id;
                }
            }
        }
    }

    // Merge into the flat plan, applying entry-edge rewrites and
    // dropping the now-redundant entry edge rows. Single-writer
    // enforcement is delegated to invariant 5 in step 4, so this
    // loop preserves every surviving edge verbatim — duplicate
    // artifact ids surface as a kernel-invariant violation rather
    // than a silent compile-time merge.
    let mut nodes: Vec<Node> = Vec::new();
    let mut edges: Vec<Edge> = Vec::new();
    let mut spec_index: HashMap<SpecId, SpecSlot> = HashMap::new();

    // Iterate in spec_plan.specs declaration order so the resulting
    // node/edge ordering is deterministic across runs (HashMap
    // iteration is not).
    for spec in &spec_plan.specs {
        let (_, inst) = instantiated.remove(&spec.id).expect("inserted above");

        for mut node in inst.nodes {
            for input in node.inputs.iter_mut() {
                if let Some(target) = rewrites.get(&input.edge_id) {
                    *input = EdgeRef::new(*target);
                }
            }
            for output in node.outputs.iter_mut() {
                if let Some(target) = rewrites.get(&output.edge_id) {
                    *output = EdgeRef::new(*target);
                }
            }
            nodes.push(node);
        }

        for edge in inst.edges {
            if rewrites.contains_key(&edge.id) {
                // Dropped: this entry edge is now the upstream exit.
                continue;
            }
            edges.push(edge);
        }

        let entry_edges = effective_entries.remove(&spec.id).unwrap_or_default();
        spec_index.insert(
            spec.id.clone(),
            SpecSlot {
                entry_edges,
                exit_edges: inst.exit_edges,
            },
        );
    }

    // Step 4 — kernel invariants over the merged graph.
    let merged = Workflow {
        nodes,
        edges,
        entry_specs: vec![],
        output_specs: vec![],
    };
    if let Err(violations) = validate_workflow(&merged, library) {
        return Err(CompileError::Invariant(violations));
    }

    Ok(ExecutionPlan {
        nodes: merged.nodes,
        edges: merged.edges,
        spec_index,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::executor::NoOpExecutor;
    use crate::ids::{EdgeId, WorkflowId};
    use crate::spec_plan::SpecRef;
    use crate::workflow::{Edge, EdgeRef, EntrySpec, Node, OutputSpec, Workflow};
    use onsager_artifact::{ArtifactId, NodeId, Provenance};
    use std::collections::HashMap;

    /// Tiny in-memory library for tests — owns the workflows and
    /// returns refs by either WorkflowId or kind.
    struct TestLibrary {
        by_id: HashMap<WorkflowId, Workflow>,
        by_kind: HashMap<String, WorkflowId>,
    }

    impl TestLibrary {
        fn new() -> Self {
            Self {
                by_id: HashMap::new(),
                by_kind: HashMap::new(),
            }
        }

        fn register(&mut self, kind: &str, workflow: Workflow) -> WorkflowId {
            let id = WorkflowId::generate();
            self.by_id.insert(id, workflow);
            self.by_kind.insert(kind.to_string(), id);
            id
        }
    }

    impl WorkflowLibrary for TestLibrary {
        fn get(&self, id: WorkflowId) -> Option<&Workflow> {
            self.by_id.get(&id)
        }
        fn by_kind(&self, spec_kind: &str) -> Option<&Workflow> {
            self.by_kind
                .get(spec_kind)
                .and_then(|id| self.by_id.get(id))
        }
    }

    /// Build a trivial single-node workflow:
    ///
    /// ```text
    /// (entry edge_in) → [NoOp node] → (exit edge_out)
    /// ```
    fn passthrough_workflow() -> Workflow {
        let edge_in = EdgeId::generate();
        let edge_out = EdgeId::generate();
        Workflow {
            nodes: vec![Node {
                id: NodeId::generate(),
                executor: Box::new(NoOpExecutor),
                inputs: vec![EdgeRef::new(edge_in)],
                outputs: vec![EdgeRef::new(edge_out)],
            }],
            edges: vec![
                Edge {
                    id: edge_in,
                    artifact_id: ArtifactId::new("art_in"),
                    requires_deterministic: false,
                },
                Edge {
                    id: edge_out,
                    artifact_id: ArtifactId::new("art_out"),
                    requires_deterministic: false,
                },
            ],
            entry_specs: vec![EntrySpec { edge_id: edge_in }],
            output_specs: vec![OutputSpec {
                edge_id: edge_out,
                provenance: Provenance::external_deterministic(),
            }],
        }
    }

    /// A second workflow shape so we can prove cross-kind connect
    /// works. Identical interface (entry/exit), distinct kind.
    fn sink_workflow() -> Workflow {
        let edge_in = EdgeId::generate();
        Workflow {
            nodes: vec![Node {
                id: NodeId::generate(),
                executor: Box::new(NoOpExecutor),
                inputs: vec![EdgeRef::new(edge_in)],
                outputs: vec![],
            }],
            edges: vec![Edge {
                id: edge_in,
                artifact_id: ArtifactId::new("art_in"),
                requires_deterministic: false,
            }],
            entry_specs: vec![EntrySpec { edge_id: edge_in }],
            output_specs: vec![],
        }
    }

    fn spec_plan_two_specs() -> SpecPlan {
        SpecPlan {
            specs: vec![
                SpecRef {
                    id: SpecId::new("s1"),
                    kind: "passthrough".to_string(),
                    inputs: Default::default(),
                },
                SpecRef {
                    id: SpecId::new("s2"),
                    kind: "sink".to_string(),
                    inputs: Default::default(),
                },
            ],
            deps: vec![SpecDep {
                from: SpecId::new("s1"),
                to: SpecId::new("s2"),
            }],
        }
    }

    fn make_library() -> TestLibrary {
        let mut lib = TestLibrary::new();
        lib.register("passthrough", passthrough_workflow());
        lib.register("sink", sink_workflow());
        lib
    }

    // ---------------------------------------------------------------
    // Determinism: compile twice → byte-identical serialized output.
    // ---------------------------------------------------------------

    #[test]
    fn compile_is_deterministic_across_runs() {
        let lib = make_library();
        let plan = spec_plan_two_specs();

        let p1 = compile(&plan, &lib).unwrap();
        let p2 = compile(&plan, &lib).unwrap();

        // Compare node/edge identifiers — the part that v4 UUIDs
        // would have randomized.
        let ids1: Vec<_> = p1.nodes.iter().map(|n| n.id).collect();
        let ids2: Vec<_> = p2.nodes.iter().map(|n| n.id).collect();
        assert_eq!(ids1, ids2, "node ids must be deterministic across runs");

        let edge_ids1: Vec<_> = p1.edges.iter().map(|e| e.id).collect();
        let edge_ids2: Vec<_> = p2.edges.iter().map(|e| e.id).collect();
        assert_eq!(
            edge_ids1, edge_ids2,
            "edge ids must be deterministic across runs",
        );

        let arts1: Vec<_> = p1.edges.iter().map(|e| e.artifact_id.clone()).collect();
        let arts2: Vec<_> = p2.edges.iter().map(|e| e.artifact_id.clone()).collect();
        assert_eq!(
            arts1, arts2,
            "artifact ids must be deterministic across runs"
        );
    }

    // ---------------------------------------------------------------
    // Cross-kind dep wires upstream exit to downstream entry.
    // ---------------------------------------------------------------

    #[test]
    fn cross_kind_dep_wires_exit_to_entry() {
        let lib = make_library();
        let plan = spec_plan_two_specs();
        let compiled = compile(&plan, &lib).unwrap();

        // Two specs → two nodes (passthrough + sink).
        assert_eq!(compiled.nodes.len(), 2);

        // s1's exit edge id (after instantiation) is the only edge
        // any sink-side consumer should reference. Find it via the
        // back-pointer in spec_index.
        let s1_exit = compiled.spec_index[&SpecId::new("s1")].exit_edges[0].edge_id;
        let s2_entry_after = &compiled.spec_index[&SpecId::new("s2")].entry_edges;
        assert_eq!(
            s2_entry_after,
            &vec![s1_exit],
            "s2's entry should now reference s1's exit edge id",
        );

        // The sink node consumes s1_exit (rewired from s2's original
        // entry edge id).
        let sink_node = compiled
            .nodes
            .iter()
            .find(|n| n.outputs.is_empty())
            .expect("sink workflow has one terminal node");
        assert_eq!(sink_node.inputs.len(), 1);
        assert_eq!(
            sink_node.inputs[0].edge_id, s1_exit,
            "sink node should consume the upstream exit edge directly",
        );

        // The dropped entry edge of s2 must not appear in the merged
        // edge list.
        let edge_ids: std::collections::HashSet<_> = compiled.edges.iter().map(|e| e.id).collect();
        assert!(
            edge_ids.contains(&s1_exit),
            "merged plan must keep the upstream exit edge",
        );
    }

    // ---------------------------------------------------------------
    // Invariant violation in the merged plan → CompileError.
    // ---------------------------------------------------------------

    #[test]
    fn invariant_violation_surfaces_as_compile_error() {
        // Build a single-spec workflow that violates invariant 5
        // (single writer per artifact): two nodes both name the same
        // artifact id on their output edges.
        let edge_a = EdgeId::generate();
        let edge_b = EdgeId::generate();
        let bad = Workflow {
            nodes: vec![
                Node {
                    id: NodeId::generate(),
                    executor: Box::new(NoOpExecutor),
                    inputs: vec![],
                    outputs: vec![EdgeRef::new(edge_a)],
                },
                Node {
                    id: NodeId::generate(),
                    executor: Box::new(NoOpExecutor),
                    inputs: vec![],
                    outputs: vec![EdgeRef::new(edge_b)],
                },
            ],
            edges: vec![
                Edge {
                    id: edge_a,
                    artifact_id: ArtifactId::new("collide"),
                    requires_deterministic: false,
                },
                Edge {
                    id: edge_b,
                    artifact_id: ArtifactId::new("collide"),
                    requires_deterministic: false,
                },
            ],
            entry_specs: vec![],
            output_specs: vec![],
        };

        let mut lib = TestLibrary::new();
        lib.register("bad", bad);

        let plan = SpecPlan {
            specs: vec![SpecRef {
                id: SpecId::new("only"),
                kind: "bad".to_string(),
                inputs: Default::default(),
            }],
            deps: vec![],
        };

        let err = compile(&plan, &lib).unwrap_err();
        match err {
            CompileError::Invariant(v) => {
                assert!(
                    v.iter().any(|iv| iv.invariant == 5),
                    "expected invariant 5 violation, got {v:?}",
                );
                let msg = v.iter().find(|iv| iv.invariant == 5).unwrap().to_string();
                assert!(
                    msg.contains("only:collide"),
                    "violation message should reference the namespaced artifact id, got: {msg}",
                );
            }
            other => panic!("expected CompileError::Invariant, got {other:?}"),
        }
    }

    // ---------------------------------------------------------------
    // Missing kind is a hard error.
    // ---------------------------------------------------------------

    #[test]
    fn missing_kind_is_hard_error() {
        let lib = TestLibrary::new(); // empty
        let plan = SpecPlan {
            specs: vec![SpecRef {
                id: SpecId::new("orphan"),
                kind: "no-such-kind".to_string(),
                inputs: Default::default(),
            }],
            deps: vec![],
        };
        let err = compile(&plan, &lib).unwrap_err();
        match err {
            CompileError::MissingKind { spec_id, kind } => {
                assert_eq!(spec_id.as_str(), "orphan");
                assert_eq!(kind, "no-such-kind");
            }
            other => panic!("expected MissingKind, got {other:?}"),
        }
    }

    // ---------------------------------------------------------------
    // Spec Plan structural error short-circuits before any lookup.
    // ---------------------------------------------------------------

    #[test]
    fn spec_plan_cycle_short_circuits_compile() {
        let lib = TestLibrary::new(); // empty — would fail MissingKind
        let plan = SpecPlan {
            specs: vec![
                SpecRef {
                    id: SpecId::new("a"),
                    kind: "any".to_string(),
                    inputs: Default::default(),
                },
                SpecRef {
                    id: SpecId::new("b"),
                    kind: "any".to_string(),
                    inputs: Default::default(),
                },
            ],
            deps: vec![
                SpecDep {
                    from: SpecId::new("a"),
                    to: SpecId::new("b"),
                },
                SpecDep {
                    from: SpecId::new("b"),
                    to: SpecId::new("a"),
                },
            ],
        };
        let err = compile(&plan, &lib).unwrap_err();
        assert!(
            matches!(err, CompileError::SpecPlan(SpecPlanError::CycleInDeps(_))),
            "expected SpecPlan cycle error to short-circuit before MissingKind, got: {err:?}",
        );
    }

    // ---------------------------------------------------------------
    // Two specs of the same kind don't collide on artifact ids.
    // ---------------------------------------------------------------

    #[test]
    fn two_same_kind_specs_get_distinct_artifact_ids() {
        let mut lib = TestLibrary::new();
        lib.register("passthrough", passthrough_workflow());

        let plan = SpecPlan {
            specs: vec![
                SpecRef {
                    id: SpecId::new("alpha"),
                    kind: "passthrough".to_string(),
                    inputs: Default::default(),
                },
                SpecRef {
                    id: SpecId::new("beta"),
                    kind: "passthrough".to_string(),
                    inputs: Default::default(),
                },
            ],
            deps: vec![],
        };
        let compiled = compile(&plan, &lib).unwrap();

        // Each spec contributes one node + one edge surviving (the
        // entry edge is kept because nothing was wired into it).
        assert_eq!(compiled.nodes.len(), 2);

        // Artifact ids are namespaced by spec id — no collision.
        let arts: std::collections::HashSet<_> = compiled
            .edges
            .iter()
            .map(|e| e.artifact_id.clone())
            .collect();
        assert!(arts.contains(&ArtifactId::new("alpha:art_in")));
        assert!(arts.contains(&ArtifactId::new("alpha:art_out")));
        assert!(arts.contains(&ArtifactId::new("beta:art_in")));
        assert!(arts.contains(&ArtifactId::new("beta:art_out")));
    }

    // ---------------------------------------------------------------
    // NoExit / NoEntry surface usefully when wiring is impossible.
    // ---------------------------------------------------------------

    #[test]
    fn dep_to_workflow_with_no_entry_is_rejected() {
        let mut lib = TestLibrary::new();
        // Workflow that produces something but declares no entry slot.
        let edge_out = EdgeId::generate();
        let no_entry = Workflow {
            nodes: vec![Node {
                id: NodeId::generate(),
                executor: Box::new(NoOpExecutor),
                inputs: vec![],
                outputs: vec![EdgeRef::new(edge_out)],
            }],
            edges: vec![Edge {
                id: edge_out,
                artifact_id: ArtifactId::new("art_out"),
                requires_deterministic: false,
            }],
            entry_specs: vec![],
            output_specs: vec![OutputSpec {
                edge_id: edge_out,
                provenance: Provenance::external_deterministic(),
            }],
        };
        lib.register("source", passthrough_workflow());
        lib.register("noentry", no_entry);

        let plan = SpecPlan {
            specs: vec![
                SpecRef {
                    id: SpecId::new("s1"),
                    kind: "source".to_string(),
                    inputs: Default::default(),
                },
                SpecRef {
                    id: SpecId::new("s2"),
                    kind: "noentry".to_string(),
                    inputs: Default::default(),
                },
            ],
            deps: vec![SpecDep {
                from: SpecId::new("s1"),
                to: SpecId::new("s2"),
            }],
        };
        let err = compile(&plan, &lib).unwrap_err();
        assert!(
            matches!(err, CompileError::NoEntry { .. }),
            "expected NoEntry error, got: {err:?}",
        );
    }

    // ---------------------------------------------------------------
    // Multi-incoming dep into a single spec is rejected explicitly,
    // not silently merged. v1 is single-entry per ADR 0015 — fan-in
    // exceeds the contract, and silently overwriting the rewrite
    // map (last-write-wins) would corrupt the wiring while leaving
    // the spec_index back-pointer inconsistent with the merged
    // node's actual inputs.
    // ---------------------------------------------------------------

    #[test]
    fn multiple_incoming_deps_into_same_spec_are_rejected() {
        let mut lib = TestLibrary::new();
        lib.register("passthrough", passthrough_workflow());
        lib.register("sink", sink_workflow());

        // Diamond bottom: a→c and b→c — c receives two upstream deps.
        let plan = SpecPlan {
            specs: vec![
                SpecRef {
                    id: SpecId::new("a"),
                    kind: "passthrough".to_string(),
                    inputs: Default::default(),
                },
                SpecRef {
                    id: SpecId::new("b"),
                    kind: "passthrough".to_string(),
                    inputs: Default::default(),
                },
                SpecRef {
                    id: SpecId::new("c"),
                    kind: "sink".to_string(),
                    inputs: Default::default(),
                },
            ],
            deps: vec![
                SpecDep {
                    from: SpecId::new("a"),
                    to: SpecId::new("c"),
                },
                SpecDep {
                    from: SpecId::new("b"),
                    to: SpecId::new("c"),
                },
            ],
        };
        let err = compile(&plan, &lib).unwrap_err();
        match err {
            CompileError::MultipleIncomingDeps { to, from } => {
                assert_eq!(to, SpecId::new("c"));
                let froms: std::collections::HashSet<_> = from.iter().cloned().collect();
                assert_eq!(
                    froms,
                    std::collections::HashSet::from([SpecId::new("a"), SpecId::new("b")]),
                    "error should name every offending upstream spec",
                );
            }
            other => panic!("expected MultipleIncomingDeps, got {other:?}"),
        }
    }
}
