//! Static validation of the five kernel invariants (ADR 0018).
//!
//! [`validate_workflow`] runs all five checks against a
//! [`Workflow`] template plus a [`WorkflowLibrary`] for invariant 4,
//! and collects every violation into a single [`Vec`] so authors
//! see every problem in one pass.
//!
//! The invariants — verbatim from ADR 0018:
//!
//! 1. A `requires_deterministic: true` edge cannot accept an
//!    `Uncertain` upstream output. Verify executors (ADR 0010) are
//!    the only nodes allowed to flip Uncertain → Deterministic.
//! 2. A non-Verify node's emitted provenance is the max-uncertainty
//!    of its declared output provenance and all input provenances.
//!    Uncertain is contagious; only Verify may upgrade.
//! 3. Each [`crate::workflow::OutputSpec`] on a workflow must match
//!    the actual provenance flowing into its exit edge.
//! 4. Every [`crate::executor::Executor::subworkflow_ref`] must
//!    resolve in the supplied library, and the resolution graph
//!    must be acyclic.
//! 5. Single writer per artifact — no two nodes may name the same
//!    `ArtifactId` across their output edges.
//!
//! Entry edges (edges with no producer node in this workflow) are
//! treated as `Deterministic { source: External }`. A formal
//! `EntrySpec` is out of scope for SUB-03 and will land alongside
//! the Plan Compiler work.

use std::collections::{HashMap, HashSet};
use std::fmt;

use onsager_artifact::{ArtifactId, NodeId, Provenance};

use crate::executor::Executor;
use crate::ids::{EdgeId, WorkflowId};
use crate::library::WorkflowLibrary;
use crate::workflow::{Node, Workflow};

/// One invariant violation found during [`validate_workflow`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvariantViolation {
    /// Which of the five invariants was violated (1..=5).
    pub invariant: u8,
    /// Nodes implicated in the violation. May be empty for purely
    /// edge-scoped checks; never for invariants 1, 2, 4, 5.
    pub nodes: Vec<NodeId>,
    /// Edges implicated in the violation. Empty when the violation
    /// is purely node-scoped (invariants 2, 4).
    pub edges: Vec<EdgeId>,
    /// Human-readable description naming the offending IDs.
    pub message: String,
}

impl fmt::Display for InvariantViolation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invariant {}: {}", self.invariant, self.message)
    }
}

impl std::error::Error for InvariantViolation {}

/// Run all five kernel invariants over `workflow`, collecting every
/// violation. Returns `Ok(())` only when every check passes.
///
/// `library` is consulted by invariant 4 (SubWorkflow resolution).
/// Pass `&()` when no library is available; any SubWorkflow ref in
/// the workflow will then be reported as unresolved, which is the
/// correct behavior.
pub fn validate_workflow(
    workflow: &Workflow,
    library: &dyn WorkflowLibrary,
) -> Result<(), Vec<InvariantViolation>> {
    let mut violations = Vec::new();
    let producers = ProducerIndex::build(workflow);
    let emits = EmitsIndex::build(workflow, &producers);

    check_invariant_1_requires_deterministic(workflow, &producers, &emits, &mut violations);
    check_invariant_2_uncertain_is_contagious(workflow, &producers, &mut violations);
    check_invariant_3_output_spec_matches(workflow, &producers, &emits, &mut violations);
    check_invariant_4_subworkflow_resolves(workflow, library, &mut violations);
    check_invariant_5_single_writer(workflow, &mut violations);

    if violations.is_empty() {
        Ok(())
    } else {
        Err(violations)
    }
}

// ---------------------------------------------------------------------------
// Indexes
// ---------------------------------------------------------------------------

/// Maps `edge_id → producer node` (the node that names this edge in
/// its `outputs`). Edges with no producer in this workflow — entry
/// edges — are absent from the map.
struct ProducerIndex<'w> {
    by_edge: HashMap<EdgeId, &'w Node>,
}

impl<'w> ProducerIndex<'w> {
    fn build(workflow: &'w Workflow) -> Self {
        let mut by_edge = HashMap::new();
        for node in &workflow.nodes {
            for output in &node.outputs {
                // Invariant 5 detects collisions separately; here we
                // keep the first producer so other checks still see
                // a deterministic owner per edge.
                by_edge.entry(output.edge_id).or_insert(node);
            }
        }
        Self { by_edge }
    }

    fn producer_of(&self, edge_id: EdgeId) -> Option<&'w Node> {
        self.by_edge.get(&edge_id).copied()
    }
}

/// Maps `node_id → emitted Provenance`, computed per the invariant 2
/// rule: max-uncertainty of declared + all inputs for non-Verify
/// executors; declared verbatim for Verify.
///
/// Entry edges contribute `Provenance::external_deterministic()`,
/// since the workflow does not yet carry an EntrySpec to override
/// that default.
struct EmitsIndex {
    by_node: HashMap<NodeId, Provenance>,
}

impl EmitsIndex {
    fn build(workflow: &Workflow, producers: &ProducerIndex<'_>) -> Self {
        let mut by_node = HashMap::new();
        for node in &workflow.nodes {
            let inputs = collect_input_provenances(node, producers, &by_node);
            let declared = node.executor.declared_provenance(&inputs);
            let emitted = if is_verify(node.executor.as_ref()) {
                declared
            } else {
                propagate_max_uncertainty(declared, &inputs)
            };
            by_node.insert(node.id, emitted);
        }
        Self { by_node }
    }

    fn emitted_by(&self, node_id: NodeId) -> Option<Provenance> {
        self.by_node.get(&node_id).copied()
    }
}

fn collect_input_provenances(
    node: &Node,
    producers: &ProducerIndex<'_>,
    emits_so_far: &HashMap<NodeId, Provenance>,
) -> Vec<Provenance> {
    node.inputs
        .iter()
        .map(|input| match producers.producer_of(input.edge_id) {
            Some(producer) => emits_so_far
                .get(&producer.id)
                .copied()
                // Producer not yet visited (cyclic graph or
                // out-of-order). Fall back to External
                // deterministic; cycle-detection in the workflow
                // graph is not in this issue's scope.
                .unwrap_or_else(Provenance::external_deterministic),
            None => Provenance::external_deterministic(),
        })
        .collect()
}

fn propagate_max_uncertainty(declared: Provenance, inputs: &[Provenance]) -> Provenance {
    if declared.is_uncertain() {
        return declared;
    }
    if let Some(uncertain_input) = inputs.iter().copied().find(Provenance::is_uncertain) {
        return Provenance::Uncertain {
            source: uncertain_input.source(),
        };
    }
    declared
}

fn is_verify(executor: &dyn Executor) -> bool {
    executor.executor_kind() == "verify"
}

// ---------------------------------------------------------------------------
// Invariant 1
// ---------------------------------------------------------------------------

fn check_invariant_1_requires_deterministic(
    workflow: &Workflow,
    producers: &ProducerIndex<'_>,
    emits: &EmitsIndex,
    violations: &mut Vec<InvariantViolation>,
) {
    for edge in &workflow.edges {
        if !edge.requires_deterministic {
            continue;
        }
        let Some(producer) = producers.producer_of(edge.id) else {
            // Entry edge: External default is Deterministic, so the
            // contract holds trivially.
            continue;
        };
        let Some(emitted) = emits.emitted_by(producer.id) else {
            continue;
        };
        if emitted.is_uncertain() {
            violations.push(InvariantViolation {
                invariant: 1,
                nodes: vec![producer.id],
                edges: vec![edge.id],
                message: format!(
                    "edge {} requires deterministic input but producer node {} emits Uncertain (source {})",
                    edge.id,
                    producer.id,
                    emitted.source(),
                ),
            });
        }
    }
}

// ---------------------------------------------------------------------------
// Invariant 2
// ---------------------------------------------------------------------------

fn check_invariant_2_uncertain_is_contagious(
    workflow: &Workflow,
    producers: &ProducerIndex<'_>,
    violations: &mut Vec<InvariantViolation>,
) {
    // Use a temporary emits-so-far map so each node's declaration is
    // checked against the same upstream view EmitsIndex would see —
    // we cannot reuse EmitsIndex itself because it already collapses
    // declared into the propagated value for non-Verify executors.
    let mut upstream: HashMap<NodeId, Provenance> = HashMap::new();
    for node in &workflow.nodes {
        let inputs = collect_input_provenances(node, producers, &upstream);
        let declared = node.executor.declared_provenance(&inputs);
        let any_uncertain_input = inputs.iter().any(Provenance::is_uncertain);
        if any_uncertain_input && !is_verify(node.executor.as_ref()) && !declared.is_uncertain() {
            violations.push(InvariantViolation {
                invariant: 2,
                nodes: vec![node.id],
                edges: vec![],
                message: format!(
                    "node {} ({}) declares Deterministic output despite an Uncertain input — only Verify executors may upgrade",
                    node.id,
                    node.executor.executor_kind(),
                ),
            });
        }
        upstream.insert(
            node.id,
            if is_verify(node.executor.as_ref()) {
                declared
            } else {
                propagate_max_uncertainty(declared, &inputs)
            },
        );
    }
}

// ---------------------------------------------------------------------------
// Invariant 3
// ---------------------------------------------------------------------------

fn check_invariant_3_output_spec_matches(
    workflow: &Workflow,
    producers: &ProducerIndex<'_>,
    emits: &EmitsIndex,
    violations: &mut Vec<InvariantViolation>,
) {
    for spec in &workflow.output_specs {
        let Some(producer) = producers.producer_of(spec.edge_id) else {
            violations.push(InvariantViolation {
                invariant: 3,
                nodes: vec![],
                edges: vec![spec.edge_id],
                message: format!(
                    "output spec names edge {} but no node in the workflow produces it",
                    spec.edge_id,
                ),
            });
            continue;
        };
        let Some(actual) = emits.emitted_by(producer.id) else {
            continue;
        };
        if actual != spec.provenance {
            violations.push(InvariantViolation {
                invariant: 3,
                nodes: vec![producer.id],
                edges: vec![spec.edge_id],
                message: format!(
                    "output spec on edge {} declares {} but producer node {} emits {}",
                    spec.edge_id,
                    format_provenance(spec.provenance),
                    producer.id,
                    format_provenance(actual),
                ),
            });
        }
    }
}

fn format_provenance(p: Provenance) -> String {
    let kind = if p.is_uncertain() {
        "Uncertain"
    } else {
        "Deterministic"
    };
    format!("{kind}(source={})", p.source())
}

// ---------------------------------------------------------------------------
// Invariant 4
// ---------------------------------------------------------------------------

fn check_invariant_4_subworkflow_resolves(
    workflow: &Workflow,
    library: &dyn WorkflowLibrary,
    violations: &mut Vec<InvariantViolation>,
) {
    for node in &workflow.nodes {
        let Some(target) = node.executor.subworkflow_ref() else {
            continue;
        };
        match library.get(target) {
            None => violations.push(InvariantViolation {
                invariant: 4,
                nodes: vec![node.id],
                edges: vec![],
                message: format!(
                    "node {} references SubWorkflow {} which is not registered in the library",
                    node.id, target,
                ),
            }),
            Some(_) => {
                if let Some(cycle) = detect_cycle_from(target, library) {
                    violations.push(InvariantViolation {
                        invariant: 4,
                        nodes: vec![node.id],
                        edges: vec![],
                        message: format!(
                            "node {} references SubWorkflow {} which participates in a cycle: {}",
                            node.id,
                            target,
                            format_cycle(&cycle),
                        ),
                    });
                }
            }
        }
    }
}

fn detect_cycle_from(start: WorkflowId, library: &dyn WorkflowLibrary) -> Option<Vec<WorkflowId>> {
    let mut path: Vec<WorkflowId> = Vec::new();
    let mut on_stack: HashSet<WorkflowId> = HashSet::new();
    let mut visited: HashSet<WorkflowId> = HashSet::new();
    if dfs_cycle(start, library, &mut path, &mut on_stack, &mut visited) {
        Some(path)
    } else {
        None
    }
}

fn dfs_cycle(
    current: WorkflowId,
    library: &dyn WorkflowLibrary,
    path: &mut Vec<WorkflowId>,
    on_stack: &mut HashSet<WorkflowId>,
    visited: &mut HashSet<WorkflowId>,
) -> bool {
    if on_stack.contains(&current) {
        path.push(current);
        return true;
    }
    if !visited.insert(current) {
        return false;
    }
    on_stack.insert(current);
    path.push(current);
    if let Some(workflow) = library.get(current) {
        for node in &workflow.nodes {
            if let Some(next) = node.executor.subworkflow_ref()
                && dfs_cycle(next, library, path, on_stack, visited)
            {
                return true;
            }
        }
    }
    on_stack.remove(&current);
    path.pop();
    false
}

fn format_cycle(path: &[WorkflowId]) -> String {
    path.iter()
        .map(WorkflowId::to_string)
        .collect::<Vec<_>>()
        .join(" -> ")
}

// ---------------------------------------------------------------------------
// Invariant 5
// ---------------------------------------------------------------------------

fn check_invariant_5_single_writer(workflow: &Workflow, violations: &mut Vec<InvariantViolation>) {
    let mut writers: HashMap<ArtifactId, Vec<(NodeId, EdgeId)>> = HashMap::new();
    for node in &workflow.nodes {
        for output in &node.outputs {
            let Some(edge) = workflow.edges.iter().find(|e| e.id == output.edge_id) else {
                continue;
            };
            writers
                .entry(edge.artifact_id.clone())
                .or_default()
                .push((node.id, edge.id));
        }
    }
    for (artifact_id, entries) in writers {
        let distinct_nodes: HashSet<NodeId> = entries.iter().map(|(n, _)| *n).collect();
        if distinct_nodes.len() > 1 {
            violations.push(InvariantViolation {
                invariant: 5,
                nodes: distinct_nodes.into_iter().collect(),
                edges: entries.iter().map(|(_, e)| *e).collect(),
                message: format!(
                    "artifact {} has {} distinct producer nodes; each artifact must have a single writer",
                    artifact_id,
                    entries.len(),
                ),
            });
        }
    }
}

// ---------------------------------------------------------------------------
// Stable ordering helpers — unused here today; kept private but
// available if a future check needs a deterministic violation order.
// ---------------------------------------------------------------------------

// (intentionally none — violation order today follows graph order;
// callers that care about ordering should sort by `(invariant,
// nodes, edges)`.)

#[cfg(test)]
mod tests {
    use super::*;
    use crate::executor::NoOpExecutor;
    use crate::workflow::{Edge, EdgeRef, Node, OutputSpec};
    use onsager_artifact::SourceTag;
    use serde::{Deserialize, Serialize};
    use std::collections::HashMap;

    // -----------------------------------------------------------------
    // Test fixtures: a handful of typetag-registered executors that
    // expose specific provenance behaviors the invariants discriminate
    // against. Defined here so the substrate's production catalog
    // (`NoOpExecutor`) stays minimal.
    // -----------------------------------------------------------------

    /// Always declares `Uncertain { source: Agent }` regardless of
    /// inputs — stand-in for an Agent / LLM executor.
    #[derive(Debug, Default, Serialize, Deserialize)]
    struct AlwaysUncertainExecutor;

    #[typetag::serde(name = "test-always-uncertain")]
    impl Executor for AlwaysUncertainExecutor {
        fn executor_kind(&self) -> &'static str {
            "test-always-uncertain"
        }
        fn declared_provenance(&self, _inputs: &[Provenance]) -> Provenance {
            Provenance::Uncertain {
                source: SourceTag::Agent,
            }
        }
    }

    /// Always declares `Deterministic { source: Script }` regardless
    /// of inputs — used to construct invariant 2 violations (claims
    /// Deterministic while consuming Uncertain).
    #[derive(Debug, Default, Serialize, Deserialize)]
    struct AlwaysDeterministicExecutor;

    #[typetag::serde(name = "test-always-deterministic")]
    impl Executor for AlwaysDeterministicExecutor {
        fn executor_kind(&self) -> &'static str {
            "test-always-deterministic"
        }
        fn declared_provenance(&self, _inputs: &[Provenance]) -> Provenance {
            Provenance::Deterministic {
                source: SourceTag::Script,
            }
        }
    }

    /// Stand-in for the Verify executor (EXE-04). Returns
    /// `Deterministic { source: Script }` regardless of inputs, and
    /// crucially reports `executor_kind() == "verify"` so the kernel
    /// applies the Verify exemption from invariant 2.
    #[derive(Debug, Default, Serialize, Deserialize)]
    struct VerifyExecutor;

    #[typetag::serde(name = "verify")]
    impl Executor for VerifyExecutor {
        fn executor_kind(&self) -> &'static str {
            "verify"
        }
        fn declared_provenance(&self, _inputs: &[Provenance]) -> Provenance {
            Provenance::Deterministic {
                source: SourceTag::Script,
            }
        }
    }

    /// Stand-in for the SubWorkflow executor (EXE-06). Carries the
    /// target `WorkflowId` so invariant 4 has something to resolve.
    #[derive(Debug, Serialize, Deserialize)]
    struct SubWorkflowExecutor {
        target: WorkflowId,
    }

    #[typetag::serde(name = "test-subworkflow")]
    impl Executor for SubWorkflowExecutor {
        fn executor_kind(&self) -> &'static str {
            "test-subworkflow"
        }
        fn declared_provenance(&self, inputs: &[Provenance]) -> Provenance {
            inputs
                .iter()
                .copied()
                .find(Provenance::is_uncertain)
                .unwrap_or(Provenance::external_deterministic())
        }
        fn subworkflow_ref(&self) -> Option<WorkflowId> {
            Some(self.target)
        }
    }

    struct MapLibrary(HashMap<WorkflowId, Workflow>);

    impl WorkflowLibrary for MapLibrary {
        fn get(&self, id: WorkflowId) -> Option<&Workflow> {
            self.0.get(&id)
        }
    }

    fn make_edge(req_det: bool, artifact: &str) -> Edge {
        Edge {
            id: EdgeId::generate(),
            artifact_id: ArtifactId::new(artifact),
            requires_deterministic: req_det,
        }
    }

    // -----------------------------------------------------------------
    // Invariant 1 — requires_deterministic edges reject Uncertain.
    // -----------------------------------------------------------------

    #[test]
    fn invariant_1_passes_when_producer_emits_deterministic() {
        // NoOp consuming nothing emits external-deterministic; the
        // downstream edge can require deterministic without issue.
        let edge_out = make_edge(true, "art");
        let w = Workflow {
            nodes: vec![Node {
                id: NodeId::generate(),
                executor: Box::new(NoOpExecutor),
                inputs: vec![],
                outputs: vec![EdgeRef::new(edge_out.id)],
            }],
            edges: vec![edge_out],
            output_specs: vec![],
        };
        validate_workflow(&w, &()).unwrap();
    }

    #[test]
    fn invariant_1_fails_when_agent_feeds_requires_deterministic() {
        // Agent → requires_deterministic edge → trivial downstream
        // node. Producer emits Uncertain; the edge insists on
        // Deterministic; invariant 1 fires.
        let agent_out = make_edge(true, "art_agent");
        let downstream_in = agent_out.id;
        let agent_id = NodeId::generate();
        let downstream_id = NodeId::generate();
        let w = Workflow {
            nodes: vec![
                Node {
                    id: agent_id,
                    executor: Box::new(AlwaysUncertainExecutor),
                    inputs: vec![],
                    outputs: vec![EdgeRef::new(agent_out.id)],
                },
                Node {
                    id: downstream_id,
                    executor: Box::new(NoOpExecutor),
                    inputs: vec![EdgeRef::new(downstream_in)],
                    outputs: vec![],
                },
            ],
            edges: vec![agent_out],
            output_specs: vec![],
        };
        let err = validate_workflow(&w, &()).unwrap_err();
        let v1: Vec<_> = err.iter().filter(|v| v.invariant == 1).collect();
        assert_eq!(v1.len(), 1, "expected exactly one invariant 1 violation");
        let v = v1[0];
        assert!(v.nodes.contains(&agent_id));
        assert_eq!(v.edges, vec![downstream_in]);
        assert!(
            v.message.contains(&agent_id.to_string()),
            "message should name the offending node: {}",
            v.message
        );
    }

    // -----------------------------------------------------------------
    // Invariant 2 — Uncertain is contagious via emits_provenance.
    // -----------------------------------------------------------------

    #[test]
    fn invariant_2_passes_when_verify_upgrades_uncertain_to_deterministic() {
        let agent_out = make_edge(false, "art_agent");
        let verify_out = make_edge(true, "art_verified");
        let agent_id = NodeId::generate();
        let verify_id = NodeId::generate();
        let w = Workflow {
            nodes: vec![
                Node {
                    id: agent_id,
                    executor: Box::new(AlwaysUncertainExecutor),
                    inputs: vec![],
                    outputs: vec![EdgeRef::new(agent_out.id)],
                },
                Node {
                    id: verify_id,
                    executor: Box::new(VerifyExecutor),
                    inputs: vec![EdgeRef::new(agent_out.id)],
                    outputs: vec![EdgeRef::new(verify_out.id)],
                },
            ],
            edges: vec![agent_out, verify_out],
            output_specs: vec![],
        };
        validate_workflow(&w, &()).unwrap();
    }

    #[test]
    fn invariant_2_fails_when_non_verify_declares_deterministic_over_uncertain() {
        // Agent emits Uncertain → script-executor consumes it and
        // declares Deterministic. Non-Verify; invariant 2 fires.
        let agent_out = make_edge(false, "art_agent");
        let script_out = make_edge(false, "art_script");
        let agent_id = NodeId::generate();
        let script_id = NodeId::generate();
        let w = Workflow {
            nodes: vec![
                Node {
                    id: agent_id,
                    executor: Box::new(AlwaysUncertainExecutor),
                    inputs: vec![],
                    outputs: vec![EdgeRef::new(agent_out.id)],
                },
                Node {
                    id: script_id,
                    executor: Box::new(AlwaysDeterministicExecutor),
                    inputs: vec![EdgeRef::new(agent_out.id)],
                    outputs: vec![EdgeRef::new(script_out.id)],
                },
            ],
            edges: vec![agent_out, script_out],
            output_specs: vec![],
        };
        let err = validate_workflow(&w, &()).unwrap_err();
        let v2: Vec<_> = err.iter().filter(|v| v.invariant == 2).collect();
        assert_eq!(v2.len(), 1);
        let v = v2[0];
        assert_eq!(v.nodes, vec![script_id]);
        assert!(
            v.message.contains(&script_id.to_string()),
            "message should name offending node: {}",
            v.message
        );
        assert!(
            v.message.contains("test-always-deterministic"),
            "message should name executor kind: {}",
            v.message
        );
    }

    // -----------------------------------------------------------------
    // Invariant 3 — Workflow OutputSpec matches actual provenance.
    // -----------------------------------------------------------------

    #[test]
    fn invariant_3_passes_when_output_spec_matches_emitted() {
        let out_edge = make_edge(false, "art_out");
        let producer = NodeId::generate();
        let spec = OutputSpec {
            edge_id: out_edge.id,
            provenance: Provenance::external_deterministic(),
        };
        let w = Workflow {
            nodes: vec![Node {
                id: producer,
                executor: Box::new(NoOpExecutor),
                inputs: vec![],
                outputs: vec![EdgeRef::new(out_edge.id)],
            }],
            edges: vec![out_edge],
            output_specs: vec![spec],
        };
        validate_workflow(&w, &()).unwrap();
    }

    #[test]
    fn invariant_3_fails_when_output_spec_promises_deterministic_but_emits_uncertain() {
        let out_edge = make_edge(false, "art_out");
        let agent_id = NodeId::generate();
        let spec = OutputSpec {
            edge_id: out_edge.id,
            provenance: Provenance::Deterministic {
                source: SourceTag::Script,
            },
        };
        let w = Workflow {
            nodes: vec![Node {
                id: agent_id,
                executor: Box::new(AlwaysUncertainExecutor),
                inputs: vec![],
                outputs: vec![EdgeRef::new(out_edge.id)],
            }],
            edges: vec![out_edge.clone()],
            output_specs: vec![spec],
        };
        let err = validate_workflow(&w, &()).unwrap_err();
        let v3: Vec<_> = err.iter().filter(|v| v.invariant == 3).collect();
        assert_eq!(v3.len(), 1);
        let v = v3[0];
        assert_eq!(v.nodes, vec![agent_id]);
        assert_eq!(v.edges, vec![out_edge.id]);
        assert!(
            v.message.contains(&out_edge.id.to_string()),
            "message should name offending edge: {}",
            v.message
        );
    }

    // -----------------------------------------------------------------
    // Invariant 4 — SubWorkflow workflow_ref must resolve.
    // -----------------------------------------------------------------

    #[test]
    fn invariant_4_passes_when_subworkflow_ref_resolves_in_library() {
        let target_id = WorkflowId::generate();
        let mut map = HashMap::new();
        map.insert(target_id, Workflow::default());
        let lib = MapLibrary(map);

        let w = Workflow {
            nodes: vec![Node {
                id: NodeId::generate(),
                executor: Box::new(SubWorkflowExecutor { target: target_id }),
                inputs: vec![],
                outputs: vec![],
            }],
            edges: vec![],
            output_specs: vec![],
        };
        validate_workflow(&w, &lib).unwrap();
    }

    #[test]
    fn invariant_4_fails_when_subworkflow_ref_does_not_resolve() {
        let missing = WorkflowId::generate();
        let caller = NodeId::generate();
        let w = Workflow {
            nodes: vec![Node {
                id: caller,
                executor: Box::new(SubWorkflowExecutor { target: missing }),
                inputs: vec![],
                outputs: vec![],
            }],
            edges: vec![],
            output_specs: vec![],
        };
        let err = validate_workflow(&w, &()).unwrap_err();
        let v4: Vec<_> = err.iter().filter(|v| v.invariant == 4).collect();
        assert_eq!(v4.len(), 1);
        let v = v4[0];
        assert_eq!(v.nodes, vec![caller]);
        assert!(
            v.message.contains(&missing.to_string()),
            "message should name unresolved workflow id: {}",
            v.message
        );
    }

    #[test]
    fn invariant_4_detects_cycle_in_subworkflow_library() {
        // wid_a -> wid_b -> wid_a (cycle between two library
        // workflows). Root workflow refers to wid_a; the validator
        // walks into the library and trips the cycle detector.
        let wid_a = WorkflowId::generate();
        let wid_b = WorkflowId::generate();
        let mut map = HashMap::new();
        map.insert(
            wid_a,
            Workflow {
                nodes: vec![Node {
                    id: NodeId::generate(),
                    executor: Box::new(SubWorkflowExecutor { target: wid_b }),
                    inputs: vec![],
                    outputs: vec![],
                }],
                edges: vec![],
                output_specs: vec![],
            },
        );
        map.insert(
            wid_b,
            Workflow {
                nodes: vec![Node {
                    id: NodeId::generate(),
                    executor: Box::new(SubWorkflowExecutor { target: wid_a }),
                    inputs: vec![],
                    outputs: vec![],
                }],
                edges: vec![],
                output_specs: vec![],
            },
        );
        let lib = MapLibrary(map);

        let caller = NodeId::generate();
        let root = Workflow {
            nodes: vec![Node {
                id: caller,
                executor: Box::new(SubWorkflowExecutor { target: wid_a }),
                inputs: vec![],
                outputs: vec![],
            }],
            edges: vec![],
            output_specs: vec![],
        };
        let err = validate_workflow(&root, &lib).unwrap_err();
        let v4: Vec<_> = err.iter().filter(|v| v.invariant == 4).collect();
        assert_eq!(v4.len(), 1, "expected exactly one cycle violation");
        assert!(
            v4[0].message.contains("cycle"),
            "message should mention cycle: {}",
            v4[0].message
        );
    }

    // -----------------------------------------------------------------
    // Invariant 5 — single writer per artifact.
    // -----------------------------------------------------------------

    #[test]
    fn invariant_5_passes_when_each_artifact_has_one_writer() {
        let edge_a = make_edge(false, "art_a");
        let edge_b = make_edge(false, "art_b");
        let w = Workflow {
            nodes: vec![
                Node {
                    id: NodeId::generate(),
                    executor: Box::new(NoOpExecutor),
                    inputs: vec![],
                    outputs: vec![EdgeRef::new(edge_a.id)],
                },
                Node {
                    id: NodeId::generate(),
                    executor: Box::new(NoOpExecutor),
                    inputs: vec![],
                    outputs: vec![EdgeRef::new(edge_b.id)],
                },
            ],
            edges: vec![edge_a, edge_b],
            output_specs: vec![],
        };
        validate_workflow(&w, &()).unwrap();
    }

    #[test]
    fn invariant_5_fails_when_two_nodes_share_an_output_artifact_id() {
        // Two distinct edges, same artifact_id — both nodes claim
        // to write art_shared.
        let edge_a = Edge {
            id: EdgeId::generate(),
            artifact_id: ArtifactId::new("art_shared"),
            requires_deterministic: false,
        };
        let edge_b = Edge {
            id: EdgeId::generate(),
            artifact_id: ArtifactId::new("art_shared"),
            requires_deterministic: false,
        };
        let n1 = NodeId::generate();
        let n2 = NodeId::generate();
        let w = Workflow {
            nodes: vec![
                Node {
                    id: n1,
                    executor: Box::new(NoOpExecutor),
                    inputs: vec![],
                    outputs: vec![EdgeRef::new(edge_a.id)],
                },
                Node {
                    id: n2,
                    executor: Box::new(NoOpExecutor),
                    inputs: vec![],
                    outputs: vec![EdgeRef::new(edge_b.id)],
                },
            ],
            edges: vec![edge_a, edge_b],
            output_specs: vec![],
        };
        let err = validate_workflow(&w, &()).unwrap_err();
        let v5: Vec<_> = err.iter().filter(|v| v.invariant == 5).collect();
        assert_eq!(v5.len(), 1);
        let v = v5[0];
        let nodes: HashSet<NodeId> = v.nodes.iter().copied().collect();
        assert_eq!(nodes, HashSet::from([n1, n2]));
        assert!(
            v.message.contains("art_shared"),
            "message should name the artifact id: {}",
            v.message
        );
    }

    // -----------------------------------------------------------------
    // Cross-invariant: multiple violations surface together so
    // authors can fix them in one pass.
    // -----------------------------------------------------------------

    #[test]
    fn multiple_violations_are_collected_in_one_call() {
        // One graph that trips invariants 1, 2, and 5 at once.
        let agent_out = Edge {
            id: EdgeId::generate(),
            artifact_id: ArtifactId::new("art_collide"),
            requires_deterministic: true, // invariant 1 violation
        };
        let script_out = Edge {
            id: EdgeId::generate(),
            artifact_id: ArtifactId::new("art_collide"), // invariant 5 violation
            requires_deterministic: false,
        };
        let agent_id = NodeId::generate();
        let script_id = NodeId::generate();
        let w = Workflow {
            nodes: vec![
                Node {
                    id: agent_id,
                    executor: Box::new(AlwaysUncertainExecutor),
                    inputs: vec![],
                    outputs: vec![EdgeRef::new(agent_out.id)],
                },
                Node {
                    id: script_id,
                    // Consumes Uncertain, declares Deterministic, not
                    // Verify → invariant 2 violation.
                    executor: Box::new(AlwaysDeterministicExecutor),
                    inputs: vec![EdgeRef::new(agent_out.id)],
                    outputs: vec![EdgeRef::new(script_out.id)],
                },
            ],
            edges: vec![agent_out, script_out],
            output_specs: vec![],
        };
        let err = validate_workflow(&w, &()).unwrap_err();
        let invariants: HashSet<u8> = err.iter().map(|v| v.invariant).collect();
        assert!(invariants.contains(&1), "expected invariant 1: {err:?}");
        assert!(invariants.contains(&2), "expected invariant 2: {err:?}");
        assert!(invariants.contains(&5), "expected invariant 5: {err:?}");
    }
}
