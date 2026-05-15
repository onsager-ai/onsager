//! `Workflow`, `Node`, `Edge`, `EdgeRef` — the substrate template shape.
//!
//! A `Workflow` is a *template*: a graph of nodes and edges that the
//! Plan Compiler (SUB-05, #352) instantiates against a Spec Plan
//! to produce an executable Execution Plan (SUB-04, #351).
//!
//! See [ADR 0009](../../../docs/adr/0009-three-layer-pipeline.md) for
//! the three-layer pipeline this type lives at the middle of, and
//! [ADR 0012](../../../docs/adr/0012-executor-catalog-replaces-nodekind.md)
//! for why `Node` carries `Box<dyn Executor>` instead of a closed
//! `NodeKind` enum.

use onsager_artifact::{ArtifactId, NodeId, Provenance};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::executor::Executor;
use crate::ids::EdgeId;
use crate::spec_plan::SpecId;

/// A reusable workflow template — graph of [`Node`]s connected by
/// [`Edge`]s.
///
/// Workflows are referenced by `spec_kind` in the Workflow Library
/// (SUB-04, #351). The library row carries the `WorkflowId` and
/// version; the `Workflow` struct itself is the *content* — the shape
/// the Plan Compiler instantiates.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Workflow {
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
    /// Workflow-level entry slots — each entry names an inbound edge
    /// the Plan Compiler may rewire when wiring spec-level deps
    /// (ADR 0017 step 3). v1 fixes single-entry / single-exit per
    /// ADR 0015; declaring zero entries is also valid (the workflow
    /// stands alone, with no upstream wiring).
    #[serde(default)]
    pub entry_specs: Vec<EntrySpec>,
    /// Workflow-level output slots — each entry names an exit edge
    /// and the provenance the workflow promises to deliver on it.
    /// Invariant 3 (ADR 0018) checks the actual emitted provenance
    /// on each named edge equals the declaration; a workflow may
    /// declare zero outputs, in which case invariant 3 is a no-op.
    #[serde(default)]
    pub output_specs: Vec<OutputSpec>,
}

/// A declared workflow output slot.
///
/// Pairs an exit `EdgeId` with the `Provenance` the workflow
/// promises to deliver on it. Validated by invariant 3 (ADR 0018):
/// the actual emitted provenance flowing into `edge_id` (computed
/// from the producer node's executor + inputs per invariant 2) must
/// equal `provenance`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutputSpec {
    pub edge_id: EdgeId,
    pub provenance: Provenance,
}

/// A declared workflow entry slot.
///
/// Names an inbound edge the Plan Compiler may rewire when an
/// upstream spec dependency is connected to this spec. The compiler
/// rewrites consumer references to this `edge_id` so they read from
/// the upstream spec's exit edge instead.
///
/// Standalone workflows may omit entry specs entirely; the kernel
/// validator already treats unproduced edges as External.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntrySpec {
    pub edge_id: EdgeId,
}

/// A node in a workflow template.
///
/// Behavior lives entirely in `executor`; the rest of the struct is
/// just graph wiring. `inputs` and `outputs` reference edges by ID
/// (see [`EdgeRef`]).
#[derive(Debug, Serialize, Deserialize)]
pub struct Node {
    pub id: NodeId,
    pub executor: Box<dyn Executor>,
    pub inputs: Vec<EdgeRef>,
    pub outputs: Vec<EdgeRef>,
}

/// An edge connecting two nodes in a workflow template.
///
/// `requires_deterministic` is the kernel's first invariant teeth
/// (ADR 0018 invariant 1): if `true`, the upstream node's emitted
/// provenance must be `Deterministic`, or the workflow refuses to
/// compile. Verify executors (ADR 0010) are the only path to flipping
/// `Uncertain` upstream into `Deterministic` downstream.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Edge {
    pub id: EdgeId,
    pub artifact_id: ArtifactId,
    #[serde(default)]
    pub requires_deterministic: bool,
}

/// A reference to an [`Edge`] from a [`Node`]'s `inputs` / `outputs`
/// list.
///
/// A separate type (rather than a bare `EdgeId`) so we can extend the
/// reference shape later — e.g. a `role` tag if a node ever takes
/// multiple inputs of the same artifact id — without rewriting every
/// caller. The kernel today only reads `edge_id`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EdgeRef {
    pub edge_id: EdgeId,
}

impl EdgeRef {
    /// Build a reference to the given edge.
    pub fn new(edge_id: EdgeId) -> Self {
        Self { edge_id }
    }
}

impl From<EdgeId> for EdgeRef {
    fn from(edge_id: EdgeId) -> Self {
        Self { edge_id }
    }
}

/// Result of [`Workflow::instantiate`] — a copy of the workflow's
/// node graph with deterministic, namespace-scoped identifiers.
///
/// The Plan Compiler ([`crate::compiler::compile`]) consumes one of
/// these per spec, then merges them into the flat Execution Plan and
/// rewires entry edges according to the Spec Plan deps.
#[derive(Debug)]
pub struct InstantiatedWorkflow {
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
    pub entry_edges: Vec<EdgeId>,
    pub exit_edges: Vec<OutputSpec>,
}

/// Stable namespace seed for Plan-Compiler UUID derivation. Generated
/// once (UUID v4) and frozen — changing it invalidates every cached
/// compile output. The value itself is arbitrary; only its
/// stability matters.
const PLAN_COMPILER_NAMESPACE: Uuid = Uuid::from_bytes([
    0x4f, 0x4e, 0x53, 0x47, 0x52, 0x53, 0x55, 0x42, 0x53, 0x54, 0x52, 0x41, 0x54, 0x45, 0x05, 0x05,
]);

impl Workflow {
    /// Produce a fresh copy of this workflow's nodes and edges with
    /// every `NodeId` / `EdgeId` / `ArtifactId` rewritten under
    /// `spec_id`'s namespace.
    ///
    /// Per ADR 0017, identifiers are deterministic — same `spec_id` +
    /// same `Workflow` content → byte-identical output. Achieved with
    /// UUID v5: the per-spec namespace is
    /// `Uuid::new_v5(PLAN_COMPILER_NAMESPACE, spec_id)`, and each
    /// original UUID is rewritten to
    /// `Uuid::new_v5(spec_namespace, original_uuid)`.
    ///
    /// `ArtifactId` strings are namespaced by prefixing with
    /// `"<spec_id>:"` so two specs of the same kind do not collide on
    /// the spine's single-writer-per-artifact rule (invariant 5).
    pub fn instantiate(&self, spec_id: &SpecId) -> InstantiatedWorkflow {
        let spec_ns = Uuid::new_v5(&PLAN_COMPILER_NAMESPACE, spec_id.as_str().as_bytes());

        let map_node_id = |old: NodeId| -> NodeId {
            NodeId::new(Uuid::new_v5(&spec_ns, old.as_uuid().as_bytes()))
        };
        let map_edge_id = |old: EdgeId| -> EdgeId {
            EdgeId::new(Uuid::new_v5(&spec_ns, old.as_uuid().as_bytes()))
        };
        let map_artifact_id = |old: &ArtifactId| -> ArtifactId {
            ArtifactId::new(format!("{spec_id}:{}", old.as_str()))
        };

        // Edges first — node remap reads from this re-keyed table.
        let edges: Vec<Edge> = self
            .edges
            .iter()
            .map(|edge| Edge {
                id: map_edge_id(edge.id),
                artifact_id: map_artifact_id(&edge.artifact_id),
                requires_deterministic: edge.requires_deterministic,
            })
            .collect();

        // Nodes: rewrite ids and edge refs, then re-serialize the
        // executor through serde so its trait-object form survives
        // the copy. typetag round-trips the kind discriminator.
        let nodes: Vec<Node> = self
            .nodes
            .iter()
            .map(|node| {
                let executor_json = serde_json::to_value(&node.executor)
                    .expect("Executor serializes via typetag — see crate::executor");
                let executor: Box<dyn Executor> = serde_json::from_value(executor_json)
                    .expect("Executor round-trips via typetag — kind was just emitted above");
                Node {
                    id: map_node_id(node.id),
                    executor,
                    inputs: node
                        .inputs
                        .iter()
                        .map(|r| EdgeRef::new(map_edge_id(r.edge_id)))
                        .collect(),
                    outputs: node
                        .outputs
                        .iter()
                        .map(|r| EdgeRef::new(map_edge_id(r.edge_id)))
                        .collect(),
                }
            })
            .collect();

        let entry_edges = self
            .entry_specs
            .iter()
            .map(|e| map_edge_id(e.edge_id))
            .collect();
        let exit_edges = self
            .output_specs
            .iter()
            .map(|o| OutputSpec {
                edge_id: map_edge_id(o.edge_id),
                provenance: o.provenance,
            })
            .collect();

        InstantiatedWorkflow {
            nodes,
            edges,
            entry_edges,
            exit_edges,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::executor::NoOpExecutor;

    fn single_node_workflow() -> Workflow {
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
                    requires_deterministic: true,
                },
                Edge {
                    id: edge_out,
                    artifact_id: ArtifactId::new("art_out"),
                    requires_deterministic: false,
                },
            ],
            entry_specs: vec![],
            output_specs: vec![],
        }
    }

    #[test]
    fn workflow_roundtrips_through_serde_json() {
        let original = single_node_workflow();
        let json = serde_json::to_value(&original).unwrap();
        let roundtrip: Workflow = serde_json::from_value(json).unwrap();

        // Graph shape preserved.
        assert_eq!(roundtrip.nodes.len(), 1);
        assert_eq!(roundtrip.edges.len(), 2);

        // Node identity + edge refs preserved.
        let node = &roundtrip.nodes[0];
        assert_eq!(node.id, original.nodes[0].id);
        assert_eq!(node.inputs, original.nodes[0].inputs);
        assert_eq!(node.outputs, original.nodes[0].outputs);

        // Executor round-tripped through the typetag tag — the
        // deserialized trait object reports the same kind.
        assert_eq!(node.executor.executor_kind(), "noop");

        // Edge fields preserved.
        assert_eq!(roundtrip.edges[0].id, original.edges[0].id);
        assert_eq!(
            roundtrip.edges[0].artifact_id,
            original.edges[0].artifact_id
        );
        assert!(roundtrip.edges[0].requires_deterministic);
        assert!(!roundtrip.edges[1].requires_deterministic);
    }

    #[test]
    fn workflow_executor_serializes_with_kind_discriminator() {
        let w = single_node_workflow();
        let json = serde_json::to_value(&w).unwrap();
        let exec_json = &json["nodes"][0]["executor"];
        assert_eq!(exec_json, &serde_json::json!({"kind": "noop"}));
    }

    #[test]
    fn edge_requires_deterministic_defaults_false_on_deserialize() {
        let json = serde_json::json!({
            "id": EdgeId::generate(),
            "artifact_id": "art_legacy",
        });
        let edge: Edge = serde_json::from_value(json).unwrap();
        assert!(!edge.requires_deterministic);
    }

    #[test]
    fn edge_ref_constructors_agree() {
        let edge_id = EdgeId::generate();
        assert_eq!(EdgeRef::new(edge_id), EdgeRef::from(edge_id));
        assert_eq!(EdgeRef::new(edge_id).edge_id, edge_id);
    }

    #[test]
    fn empty_workflow_roundtrips() {
        let w = Workflow::default();
        let json = serde_json::to_value(&w).unwrap();
        let roundtrip: Workflow = serde_json::from_value(json).unwrap();
        assert!(roundtrip.nodes.is_empty());
        assert!(roundtrip.edges.is_empty());
        assert!(roundtrip.output_specs.is_empty());
    }

    #[test]
    fn workflow_output_specs_default_empty_on_deserialize() {
        // Wire form predating OutputSpec must still deserialize.
        let json = serde_json::json!({"nodes": [], "edges": []});
        let w: Workflow = serde_json::from_value(json).unwrap();
        assert!(w.output_specs.is_empty());
    }

    #[test]
    fn workflow_output_specs_roundtrip() {
        let edge_id = EdgeId::generate();
        let spec = OutputSpec {
            edge_id,
            provenance: Provenance::external_deterministic(),
        };
        let w = Workflow {
            nodes: vec![],
            edges: vec![],
            entry_specs: vec![],
            output_specs: vec![spec],
        };
        let json = serde_json::to_value(&w).unwrap();
        let roundtrip: Workflow = serde_json::from_value(json).unwrap();
        assert_eq!(roundtrip.output_specs, vec![spec]);
    }
}
