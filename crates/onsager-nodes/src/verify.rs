//! [`VerifyExecutor`] (EXE-04, issue #356) — runs deterministic checks
//! against input artifacts and certifies pass/fail.
//!
//! Verify is the only kernel-recognized executor allowed to upgrade an
//! `Uncertain` input to a `Deterministic` output
//! ([ADR 0010](../../../docs/adr/0010-provenance-as-substrate-first-class.md)
//! § "Verify is the only upgrade path"). The static validator
//! ([`onsager_substrate::validate`]) keys its invariant-2 exemption on
//! `executor_kind() == "verify"`; this module is the production
//! executor that earns that exemption.
//!
//! Both halves of the
//! [ADR 0012](../../../docs/adr/0012-executor-catalog-replaces-nodekind.md)
//! Executor trait are implemented on the same type:
//!
//! - the substrate-side [`onsager_substrate::Executor`] (typetag,
//!   `kind = "verify"`) so a workflow carrying a Verify node round-
//!   trips through serde and clears the static validator;
//! - the runtime-side [`crate::Executor`] (async execute) so the
//!   scheduler can dispatch a Verify node through
//!   [`crate::ExecutorRegistry`].
//!
//! ## Scope (v1)
//!
//! This issue lands the executor *surface*, not the check
//! infrastructure. Each [`Check`] variant carries a precomputed
//! `must_pass` flag; MIG-01 (#363) swaps in the live test-runner /
//! lint / schema / governance evaluators when Synodic's gate logic is
//! ported. The kernel contract — declared provenance, invariant-2
//! exemption, fail-policy routing — is in place today.

use async_trait::async_trait;
use onsager_artifact::{Artifact, ArtifactId, Kind, NodeId, Provenance, SourceTag};
use onsager_substrate::events as se;
use onsager_substrate::executor::Executor as SubstrateExecutor;
use serde::{Deserialize, Serialize};

use crate::context::{ExecutorContext, ExecutorOutputs};
use crate::error::ExecutorError;
use crate::executor::Executor as RuntimeExecutor;

/// Wire-format tag for the Verify executor. Shared by the substrate
/// typetag discriminator and the runtime registry key — the kernel's
/// invariant-2 exemption is keyed off this exact string.
pub const VERIFY_KIND: &str = "verify";

/// What to do when one or more of [`VerifyExecutor::checks`] fails.
///
/// Mirrors the legacy `SYNODIC_FAIL_POLICY` env var (see root
/// `CLAUDE.md` § Environment variables): forge invariant #5 made
/// `Escalate` the default for missing-in-window verdicts, and the
/// same three-way choice carries forward into the substrate's
/// Verify executor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailPolicy {
    /// Surface the failure non-blockingly — return
    /// [`ExecutorError::Failed`] so the scheduler (RUN-01) parks the
    /// run and an operator decides. Default per forge invariant #5.
    #[default]
    Escalate,
    /// Hard-fail the run — same `ExecutorError::Failed` shape as
    /// `Escalate`, with a different reason string so downstream
    /// observers can distinguish ops-deny from policy-deny.
    Deny,
    /// Legacy fail-open: still emit a verified attestation (with the
    /// failure summary baked into its name) so the downstream
    /// `requires_deterministic` edge still fires. Opted into
    /// explicitly; nothing in 0.2 selects it by default.
    Allow,
}

/// One deterministic check the Verify executor runs.
///
/// The four variants preserve Synodic's gate taxonomy (test-run / lint
/// / schema / governance) so MIG-01 can route ported rules into the
/// right variant without re-encoding the wire form. v1 reads the same
/// `must_pass` flag from every variant; MIG-01 replaces this with the
/// live evaluators per kind.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Check {
    /// A test-run check — analogous to "`cargo test` passed".
    TestRun { name: String, must_pass: bool },
    /// A lint check — analogous to "`clippy` clean".
    Lint { name: String, must_pass: bool },
    /// A schema check — analogous to "migration / typecheck passed".
    Schema { name: String, must_pass: bool },
    /// A governance check — analogous to a Synodic policy verdict.
    Governance { name: String, must_pass: bool },
}

impl Check {
    fn name(&self) -> &str {
        match self {
            Check::TestRun { name, .. }
            | Check::Lint { name, .. }
            | Check::Schema { name, .. }
            | Check::Governance { name, .. } => name,
        }
    }

    fn must_pass(&self) -> bool {
        match self {
            Check::TestRun { must_pass, .. }
            | Check::Lint { must_pass, .. }
            | Check::Schema { must_pass, .. }
            | Check::Governance { must_pass, .. } => *must_pass,
        }
    }

    fn kind_tag(&self) -> &'static str {
        match self {
            Check::TestRun { .. } => "test_run",
            Check::Lint { .. } => "lint",
            Check::Schema { .. } => "schema",
            Check::Governance { .. } => "governance",
        }
    }
}

/// The Verify executor.
///
/// Carries an ordered list of [`Check`]s and a [`FailPolicy`]. Declared
/// provenance is always `Deterministic { source: Composed }` — Verify
/// *composes* its inputs into a verified attestation, which is the
/// structural reason the kernel's invariant-2 exemption is keyed on
/// `executor_kind() == "verify"`.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct VerifyExecutor {
    #[serde(default)]
    pub checks: Vec<Check>,
    #[serde(default)]
    pub fail_policy: FailPolicy,
}

impl VerifyExecutor {
    /// Empty Verify — no checks, default fail policy.
    pub fn new() -> Self {
        Self::default()
    }

    /// Build a Verify with the supplied checks and policy.
    pub fn with_checks(checks: Vec<Check>, fail_policy: FailPolicy) -> Self {
        Self {
            checks,
            fail_policy,
        }
    }

    fn verified_provenance() -> Provenance {
        Provenance::Deterministic {
            source: SourceTag::Composed,
        }
    }
}

// ---------------------------------------------------------------------------
// Substrate (static) trait — typetag round-trip + invariant exemption.
// ---------------------------------------------------------------------------

#[typetag::serde(name = "verify")]
impl SubstrateExecutor for VerifyExecutor {
    fn executor_kind(&self) -> &'static str {
        VERIFY_KIND
    }

    fn declared_provenance(&self, _inputs: &[Provenance]) -> Provenance {
        Self::verified_provenance()
    }
}

// ---------------------------------------------------------------------------
// Runtime (async) trait — registry dispatch.
// ---------------------------------------------------------------------------

#[async_trait]
impl RuntimeExecutor for VerifyExecutor {
    fn executor_kind(&self) -> &'static str {
        VERIFY_KIND
    }

    fn declared_provenance(&self, _inputs: &[Provenance]) -> Provenance {
        Self::verified_provenance()
    }

    async fn execute(&self, ctx: ExecutorContext) -> Result<ExecutorOutputs, ExecutorError> {
        let check_results: Vec<se::VerifyCheckResult> = self
            .checks
            .iter()
            .map(|c| se::VerifyCheckResult {
                name: c.name().to_string(),
                passed: c.must_pass(),
            })
            .collect();
        let passed = check_results.iter().all(|c| c.passed);

        // RUN-02 (#360): emit `synodic.verdict` for every verify
        // execution — pass or fail, escalate or deny. The dashboard
        // run timeline keys off this event, not the wrapped
        // `Err(ExecutorError)`. `plan_id` comes off the scheduler-
        // populated `ExecutorContext` so the spine envelope's
        // `stream_id` (`plan:<plan>:<node>`) correlates the verdict
        // back to a specific run.
        let verdict = se::SynodicVerdict {
            plan_id: ctx.plan_id.as_str().to_string(),
            node_id: ctx.node_id,
            passed,
            check_results,
        };
        let payload = serde_json::to_value(&verdict).expect("verdict must serialize");
        if let Err(e) = ctx.spine.emit(se::KIND_SYNODIC_VERDICT, payload).await {
            // Best-effort: a dropped verdict must not stall the run
            // (the attestation artifact still materializes), but a
            // silent drop hides why the dashboard timeline is missing
            // a row. Log the failure with enough context to find it.
            tracing::warn!(
                plan = %ctx.plan_id,
                node = %ctx.node_id,
                kind = se::KIND_SYNODIC_VERDICT,
                "verify executor spine emit failed: {e}",
            );
        }

        let failures: Vec<String> = self
            .checks
            .iter()
            .filter(|c| !c.must_pass())
            .map(|c| format!("{} ({}) did not pass", c.name(), c.kind_tag()))
            .collect();

        if failures.is_empty() {
            return Ok(ExecutorOutputs::single(
                ArtifactId::generate(),
                build_attestation(ctx.node_id, true, "all checks passed"),
            ));
        }

        let summary = failures.join("; ");
        match self.fail_policy {
            FailPolicy::Escalate => Err(ExecutorError::Failed(format!(
                "verify: escalating — {summary}"
            ))),
            FailPolicy::Deny => Err(ExecutorError::Failed(format!("verify: denied — {summary}"))),
            FailPolicy::Allow => Ok(ExecutorOutputs::single(
                ArtifactId::generate(),
                build_attestation(ctx.node_id, false, &summary),
            )),
        }
    }
}

fn build_attestation(node_id: NodeId, passed: bool, reason: &str) -> Artifact {
    let name = if passed {
        "verify.pass".to_string()
    } else {
        format!("verify.fail: {reason}")
    };
    let mut art = Artifact::new(Kind::Document, name, "kernel", "verify", vec![]);
    art.provenance = VerifyExecutor::verified_provenance();
    art.produced_by_node = Some(node_id);
    art
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spine::test_support::MockSpine;
    use std::sync::Arc;

    fn ctx_with_inputs(inputs: Vec<(ArtifactId, Artifact)>) -> ExecutorContext {
        ExecutorContext {
            plan_id: crate::scheduler::PlanId::generate(),
            node_id: NodeId::generate(),
            inputs,
            spine: Arc::new(MockSpine::default()),
        }
    }

    fn uncertain_input() -> (ArtifactId, Artifact) {
        let id = ArtifactId::generate();
        let mut art = Artifact::new(Kind::Code, "agent-edit", "marvin", "agent", vec![]);
        art.provenance = Provenance::Uncertain {
            source: SourceTag::Agent,
        };
        (id, art)
    }

    fn pass_check(name: &str) -> Check {
        Check::TestRun {
            name: name.into(),
            must_pass: true,
        }
    }

    fn fail_check(name: &str) -> Check {
        Check::Lint {
            name: name.into(),
            must_pass: false,
        }
    }

    // -----------------------------------------------------------------
    // Provenance + kind: Verify always declares Deterministic and
    // reports `executor_kind() == "verify"` on both traits.
    // -----------------------------------------------------------------

    #[test]
    fn declared_provenance_is_deterministic_composed_regardless_of_inputs() {
        let v = VerifyExecutor::new();
        let uncertain = Provenance::Uncertain {
            source: SourceTag::Agent,
        };
        let expected = Provenance::Deterministic {
            source: SourceTag::Composed,
        };
        assert_eq!(
            SubstrateExecutor::declared_provenance(&v, &[uncertain]),
            expected,
        );
        assert_eq!(
            RuntimeExecutor::declared_provenance(&v, &[uncertain]),
            expected,
        );
    }

    #[test]
    fn executor_kind_is_verify_on_both_traits() {
        let v = VerifyExecutor::new();
        assert_eq!(SubstrateExecutor::executor_kind(&v), "verify");
        assert_eq!(RuntimeExecutor::executor_kind(&v), "verify");
        assert_eq!(VERIFY_KIND, "verify");
    }

    // -----------------------------------------------------------------
    // Execute: pass / fail under each FailPolicy.
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn execute_emits_deterministic_artifact_when_all_checks_pass() {
        let v = VerifyExecutor::with_checks(
            vec![pass_check("tests"), pass_check("schema")],
            FailPolicy::Escalate,
        );
        let out = v
            .execute(ctx_with_inputs(vec![uncertain_input()]))
            .await
            .unwrap();
        assert_eq!(out.artifacts.len(), 1);
        let (_, art) = &out.artifacts[0];
        assert_eq!(
            art.provenance,
            Provenance::Deterministic {
                source: SourceTag::Composed
            },
        );
        assert!(art.name.starts_with("verify.pass"));
        assert!(art.produced_by_node.is_some());
    }

    #[tokio::test]
    async fn execute_escalates_with_failed_error_when_any_check_fails() {
        let v = VerifyExecutor::with_checks(
            vec![pass_check("tests"), fail_check("clippy")],
            FailPolicy::Escalate,
        );
        let err = v.execute(ctx_with_inputs(vec![])).await.unwrap_err();
        match err {
            ExecutorError::Failed(msg) => {
                assert!(msg.contains("escalating"), "{msg}");
                assert!(msg.contains("clippy"), "{msg}");
                assert!(msg.contains("lint"), "{msg}");
            }
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn execute_denies_with_failed_error_under_deny_policy() {
        let v = VerifyExecutor::with_checks(vec![fail_check("policy")], FailPolicy::Deny);
        let err = v.execute(ctx_with_inputs(vec![])).await.unwrap_err();
        match err {
            ExecutorError::Failed(msg) => {
                assert!(msg.contains("denied"), "{msg}");
                assert!(msg.contains("policy"), "{msg}");
            }
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn execute_still_emits_under_allow_policy_with_failure_summary() {
        let v = VerifyExecutor::with_checks(vec![fail_check("ci")], FailPolicy::Allow);
        let out = v.execute(ctx_with_inputs(vec![])).await.unwrap();
        assert_eq!(out.artifacts.len(), 1);
        let (_, art) = &out.artifacts[0];
        // Allow still flips provenance up — that's the legacy fail-
        // open semantic. The artifact name carries the failure so a
        // human reader can see what was bypassed.
        assert_eq!(
            art.provenance,
            Provenance::Deterministic {
                source: SourceTag::Composed
            },
        );
        assert!(art.name.starts_with("verify.fail"));
        assert!(art.name.contains("ci"), "{}", art.name);
    }

    // -----------------------------------------------------------------
    // Serde round-trip via typetag — the substrate trait object
    // serializes to `{"kind": "verify", ...}` and back.
    // -----------------------------------------------------------------

    #[test]
    fn verify_executor_roundtrips_as_substrate_trait_object() {
        let original: Box<dyn SubstrateExecutor> = Box::new(VerifyExecutor::with_checks(
            vec![pass_check("tests"), fail_check("clippy")],
            FailPolicy::Deny,
        ));
        let json = serde_json::to_value(&original).unwrap();
        assert_eq!(json["kind"], "verify");
        assert_eq!(json["fail_policy"], "deny");
        assert_eq!(json["checks"].as_array().unwrap().len(), 2);

        let roundtrip: Box<dyn SubstrateExecutor> = serde_json::from_value(json).unwrap();
        assert_eq!(roundtrip.executor_kind(), "verify");
    }

    // -----------------------------------------------------------------
    // Dispatch through registry — Verify resolves through the runtime
    // registry exactly like any other executor.
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn dispatch_through_registry_runs_the_verify_executor() {
        use crate::dispatch;
        use crate::registry::ExecutorRegistry;
        use onsager_substrate::workflow::Node;

        let mut registry = ExecutorRegistry::new();
        registry.register(Arc::new(VerifyExecutor::new()));

        let node = Node {
            id: NodeId::generate(),
            executor: Box::new(VerifyExecutor::new()),
            inputs: vec![],
            outputs: vec![],
        };

        let out = dispatch(&registry, &node, ctx_with_inputs(vec![]))
            .await
            .unwrap();
        assert_eq!(out.artifacts.len(), 1);
    }

    // -----------------------------------------------------------------
    // Acceptance test from issue #356:
    //   Agent (Uncertain) → Verify → Ship (requires_deterministic)
    // The static validator must clear this chain — Verify is the
    // *only* node permitted to upgrade Uncertain to Deterministic.
    // -----------------------------------------------------------------

    #[test]
    fn workflow_with_verify_upgrades_uncertain_for_requires_deterministic_edge() {
        use onsager_substrate::executor::NoOpExecutor as SubstrateNoOp;
        use onsager_substrate::ids::EdgeId;
        use onsager_substrate::validate::validate_workflow;
        use onsager_substrate::workflow::{Edge, EdgeRef, Node, Workflow};

        // Inline Agent stand-in: substrate's real Agent executor
        // (EXE-03, #355) hasn't landed, so the test declares a
        // minimal Uncertain-emitting executor here. The kernel only
        // discriminates Verify from non-Verify via `executor_kind`,
        // so any other tag suffices for the upstream node.
        #[derive(Debug, Default, Serialize, Deserialize)]
        struct AgentStub;

        #[typetag::serde(name = "test-verify-agent-stub")]
        impl SubstrateExecutor for AgentStub {
            fn executor_kind(&self) -> &'static str {
                "test-verify-agent-stub"
            }
            fn declared_provenance(&self, _inputs: &[Provenance]) -> Provenance {
                Provenance::Uncertain {
                    source: SourceTag::Agent,
                }
            }
        }

        let agent_out = Edge {
            id: EdgeId::generate(),
            artifact_id: ArtifactId::new("art_agent"),
            requires_deterministic: false,
        };
        let verify_out = Edge {
            id: EdgeId::generate(),
            artifact_id: ArtifactId::new("art_verified"),
            // Ship's input edge insists on Deterministic — only a
            // Verify in the middle can satisfy this against an
            // Uncertain agent upstream.
            requires_deterministic: true,
        };
        let agent_id = NodeId::generate();
        let verify_id = NodeId::generate();
        let ship_id = NodeId::generate();
        let w = Workflow {
            nodes: vec![
                Node {
                    id: agent_id,
                    executor: Box::new(AgentStub),
                    inputs: vec![],
                    outputs: vec![EdgeRef::new(agent_out.id)],
                },
                Node {
                    id: verify_id,
                    executor: Box::new(VerifyExecutor::new()),
                    inputs: vec![EdgeRef::new(agent_out.id)],
                    outputs: vec![EdgeRef::new(verify_out.id)],
                },
                Node {
                    id: ship_id,
                    executor: Box::new(SubstrateNoOp),
                    inputs: vec![EdgeRef::new(verify_out.id)],
                    outputs: vec![],
                },
            ],
            edges: vec![agent_out, verify_out],
            entry_specs: vec![],
            output_specs: vec![],
        };
        // Pass: invariant 1 holds because Verify emits Deterministic
        // even though its input is Uncertain.
        validate_workflow(&w, &()).expect("Agent→Verify→Ship chain must validate");
    }

    // -----------------------------------------------------------------
    // Acceptance test (negative half): a non-Verify executor downstream
    // of the Uncertain Agent cannot satisfy a `requires_deterministic`
    // edge — invariant 1 fires. Confirms the executor-kind
    // discriminator is load-bearing, not just decorative.
    // -----------------------------------------------------------------

    #[test]
    fn workflow_without_verify_fails_invariant_1_for_uncertain_upstream() {
        use onsager_substrate::executor::NoOpExecutor as SubstrateNoOp;
        use onsager_substrate::ids::EdgeId;
        use onsager_substrate::validate::validate_workflow;
        use onsager_substrate::workflow::{Edge, EdgeRef, Node, Workflow};

        #[derive(Debug, Default, Serialize, Deserialize)]
        struct AgentStubB;

        #[typetag::serde(name = "test-verify-agent-stub-b")]
        impl SubstrateExecutor for AgentStubB {
            fn executor_kind(&self) -> &'static str {
                "test-verify-agent-stub-b"
            }
            fn declared_provenance(&self, _inputs: &[Provenance]) -> Provenance {
                Provenance::Uncertain {
                    source: SourceTag::Agent,
                }
            }
        }

        let agent_out = Edge {
            id: EdgeId::generate(),
            artifact_id: ArtifactId::new("art_agent_b"),
            requires_deterministic: true,
        };
        let w = Workflow {
            nodes: vec![
                Node {
                    id: NodeId::generate(),
                    executor: Box::new(AgentStubB),
                    inputs: vec![],
                    outputs: vec![EdgeRef::new(agent_out.id)],
                },
                Node {
                    id: NodeId::generate(),
                    executor: Box::new(SubstrateNoOp),
                    inputs: vec![EdgeRef::new(agent_out.id)],
                    outputs: vec![],
                },
            ],
            edges: vec![agent_out],
            entry_specs: vec![],
            output_specs: vec![],
        };
        let err = validate_workflow(&w, &()).unwrap_err();
        assert!(err.iter().any(|v| v.invariant == 1));
    }
}
