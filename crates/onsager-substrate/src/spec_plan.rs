//! Spec Plan — the external contract authored by humans / Refract / MCP
//! clients and consumed by the Plan Compiler (SUB-05, #352).
//!
//! See [ADR 0015](../../../docs/adr/0015-spec-plan-as-dag-external-contract.md)
//! for the design rationale. A Spec Plan is a DAG: a list of [`SpecRef`]
//! nodes and a list of [`SpecDep`] edges between them. Each `SpecRef`
//! names the workflow `kind` to compile against and carries any
//! external entry-side artifact references.
//!
//! The compiler ([`crate::compiler::compile`]) walks `specs` to
//! instantiate per-spec subgraphs, then walks `deps` to wire each
//! upstream spec's exit to the downstream spec's entry. ADR 0015 fixes
//! v1 at single-entry / single-exit per workflow.
//!
//! Validation of the Spec Plan as a DAG (cycles in `deps` are an
//! error) lives in [`SpecPlan::validate`]; the compiler calls it
//! before any lookup.

use onsager_artifact::ArtifactId;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fmt;

/// Externally-assigned identity for a spec. GitHub issue number,
/// Refract-allocated UUID, `mcp:<uuid>` — anything stable and
/// stringable.
///
/// The compiler uses `SpecId` as the namespace key when instantiating
/// a workflow (see [`crate::compiler::compile`]), so renumbering
/// breaks identity per ADR 0015.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SpecId(String);

impl SpecId {
    /// Wrap an externally-assigned identifier. Empty strings are
    /// permitted at the type level — callers that need to reject
    /// them should validate at the boundary.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// The wrapped identifier as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SpecId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<String> for SpecId {
    fn from(id: String) -> Self {
        Self(id)
    }
}

impl From<&str> for SpecId {
    fn from(id: &str) -> Self {
        Self(id.to_string())
    }
}

/// Entry-side artifact references for a spec.
///
/// Today this is a flat list of `ArtifactId`s the spec expects to
/// have available at its workflow's entry edge. v1 does not type or
/// position-tag these — single-entry / single-exit per ADR 0015 means
/// at most one entry edge, and the compiler does not need to address
/// individual inputs to wire deps.
///
/// The field is here so the Spec Plan shape matches ADR 0015's
/// declared contract; downstream consumers (the scheduler, executor
/// runtime) will pick it up later.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpecInputs {
    #[serde(default)]
    pub artifacts: Vec<ArtifactId>,
}

/// A single spec in a Spec Plan.
///
/// `kind` is matched against the Workflow Library at compile time;
/// missing kinds are a hard error per ADR 0017 (no fallback).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpecRef {
    pub id: SpecId,
    pub kind: String,
    #[serde(default)]
    pub inputs: SpecInputs,
}

/// A directed dependency between two specs.
///
/// `from` produces, `to` consumes. The compiler wires `from`'s
/// workflow exit edge to `to`'s workflow entry edge; cycles among
/// `SpecDep`s are a Spec Plan validation error (ADR 0015).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpecDep {
    pub from: SpecId,
    pub to: SpecId,
}

/// The whole Spec Plan: specs + dependency edges.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SpecPlan {
    pub specs: Vec<SpecRef>,
    #[serde(default)]
    pub deps: Vec<SpecDep>,
}

/// Why a Spec Plan failed validation. The compiler surfaces these
/// before any workflow lookup so authors see structural errors
/// without having to populate the library first.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SpecPlanError {
    /// Two `SpecRef`s share the same `SpecId`.
    #[error("duplicate spec id '{0}' in Spec Plan")]
    DuplicateSpecId(SpecId),
    /// A `SpecDep` references an id that is not in `specs`.
    #[error("dependency references unknown spec id '{0}'")]
    UnknownSpecId(SpecId),
    /// `deps` contains a cycle. The vector is one cycle on the
    /// dependency graph in traversal order.
    #[error("cycle in Spec Plan deps: {}", format_cycle(.0))]
    CycleInDeps(Vec<SpecId>),
}

fn format_cycle(path: &[SpecId]) -> String {
    path.iter()
        .map(SpecId::to_string)
        .collect::<Vec<_>>()
        .join(" -> ")
}

impl SpecPlan {
    /// Run the structural checks the compiler depends on:
    /// - every `SpecId` is unique
    /// - every `SpecDep` references a known spec
    /// - `deps` is acyclic (DAG)
    pub fn validate(&self) -> Result<(), SpecPlanError> {
        let mut seen: HashSet<&SpecId> = HashSet::new();
        for spec in &self.specs {
            if !seen.insert(&spec.id) {
                return Err(SpecPlanError::DuplicateSpecId(spec.id.clone()));
            }
        }

        for dep in &self.deps {
            if !seen.contains(&dep.from) {
                return Err(SpecPlanError::UnknownSpecId(dep.from.clone()));
            }
            if !seen.contains(&dep.to) {
                return Err(SpecPlanError::UnknownSpecId(dep.to.clone()));
            }
        }

        // Adjacency map. We walk in declaration order so the cycle
        // path the user sees is reproducible run-to-run.
        let mut adj: HashMap<&SpecId, Vec<&SpecId>> = HashMap::new();
        for spec in &self.specs {
            adj.entry(&spec.id).or_default();
        }
        for dep in &self.deps {
            adj.entry(&dep.from).or_default().push(&dep.to);
        }

        let mut color: HashMap<&SpecId, Color> = HashMap::new();
        for spec in &self.specs {
            if !matches!(color.get(&spec.id), Some(Color::Black)) {
                let mut stack: Vec<&SpecId> = Vec::new();
                if dfs_cycle(&spec.id, &adj, &mut color, &mut stack) {
                    return Err(SpecPlanError::CycleInDeps(
                        stack.into_iter().cloned().collect(),
                    ));
                }
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Color {
    Gray,
    Black,
}

fn dfs_cycle<'p>(
    node: &'p SpecId,
    adj: &HashMap<&'p SpecId, Vec<&'p SpecId>>,
    color: &mut HashMap<&'p SpecId, Color>,
    stack: &mut Vec<&'p SpecId>,
) -> bool {
    color.insert(node, Color::Gray);
    stack.push(node);
    if let Some(succs) = adj.get(node) {
        for next in succs {
            match color.get(next) {
                Some(Color::Gray) => {
                    stack.push(next);
                    return true;
                }
                Some(Color::Black) => {}
                None => {
                    if dfs_cycle(next, adj, color, stack) {
                        return true;
                    }
                }
            }
        }
    }
    stack.pop();
    color.insert(node, Color::Black);
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(id: &str, kind: &str) -> SpecRef {
        SpecRef {
            id: SpecId::new(id),
            kind: kind.to_string(),
            inputs: SpecInputs::default(),
        }
    }

    fn dep(from: &str, to: &str) -> SpecDep {
        SpecDep {
            from: SpecId::new(from),
            to: SpecId::new(to),
        }
    }

    #[test]
    fn empty_plan_is_valid() {
        SpecPlan::default().validate().unwrap();
    }

    #[test]
    fn linear_chain_is_valid() {
        let plan = SpecPlan {
            specs: vec![spec("a", "k"), spec("b", "k"), spec("c", "k")],
            deps: vec![dep("a", "b"), dep("b", "c")],
        };
        plan.validate().unwrap();
    }

    #[test]
    fn duplicate_spec_id_is_rejected() {
        let plan = SpecPlan {
            specs: vec![spec("a", "k"), spec("a", "k2")],
            deps: vec![],
        };
        assert!(matches!(
            plan.validate().unwrap_err(),
            SpecPlanError::DuplicateSpecId(_)
        ));
    }

    #[test]
    fn unknown_dep_target_is_rejected() {
        let plan = SpecPlan {
            specs: vec![spec("a", "k")],
            deps: vec![dep("a", "missing")],
        };
        let err = plan.validate().unwrap_err();
        match err {
            SpecPlanError::UnknownSpecId(id) => assert_eq!(id.as_str(), "missing"),
            other => panic!("expected UnknownSpecId, got {other:?}"),
        }
    }

    #[test]
    fn cycle_is_rejected_with_path() {
        let plan = SpecPlan {
            specs: vec![spec("a", "k"), spec("b", "k"), spec("c", "k")],
            deps: vec![dep("a", "b"), dep("b", "c"), dep("c", "a")],
        };
        let err = plan.validate().unwrap_err();
        match err {
            SpecPlanError::CycleInDeps(path) => {
                let display = path
                    .iter()
                    .map(SpecId::to_string)
                    .collect::<Vec<_>>()
                    .join(",");
                assert!(
                    display.contains("a") && display.contains("b") && display.contains("c"),
                    "cycle path should include every node: {display}",
                );
            }
            other => panic!("expected CycleInDeps, got {other:?}"),
        }
    }

    #[test]
    fn self_loop_is_rejected() {
        let plan = SpecPlan {
            specs: vec![spec("a", "k")],
            deps: vec![dep("a", "a")],
        };
        assert!(matches!(
            plan.validate().unwrap_err(),
            SpecPlanError::CycleInDeps(_)
        ));
    }

    #[test]
    fn diamond_shape_is_acyclic_and_valid() {
        // a -> b, a -> c, b -> d, c -> d
        let plan = SpecPlan {
            specs: vec![
                spec("a", "k"),
                spec("b", "k"),
                spec("c", "k"),
                spec("d", "k"),
            ],
            deps: vec![dep("a", "b"), dep("a", "c"), dep("b", "d"), dep("c", "d")],
        };
        plan.validate().unwrap();
    }

    #[test]
    fn spec_plan_roundtrips_through_serde() {
        let plan = SpecPlan {
            specs: vec![SpecRef {
                id: SpecId::new("sp1"),
                kind: "github-issue".to_string(),
                inputs: SpecInputs {
                    artifacts: vec![ArtifactId::new("art1")],
                },
            }],
            deps: vec![],
        };
        let json = serde_json::to_value(&plan).unwrap();
        let roundtrip: SpecPlan = serde_json::from_value(json).unwrap();
        assert_eq!(roundtrip.specs.len(), 1);
        assert_eq!(roundtrip.specs[0].id, plan.specs[0].id);
        assert_eq!(roundtrip.specs[0].kind, "github-issue");
        assert_eq!(roundtrip.specs[0].inputs.artifacts.len(), 1);
    }

    #[test]
    fn spec_id_displays_inner_string() {
        assert_eq!(SpecId::new("github:42").to_string(), "github:42");
    }
}
