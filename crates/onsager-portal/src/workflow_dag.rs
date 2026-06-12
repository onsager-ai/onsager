//! Authored workflow definition → executable library DAG
//! (ADR 0028 / spec #602).
//!
//! The builder's stage list is the workflow's **authored** form
//! (persisted as `workflows.definition_json`); this module derives the
//! **executable** form — a substrate `Workflow` DAG — and registers it
//! in `workflow_library` under `spec_kind = <workflow_id>`, the same
//! logical→physical split the substrate itself uses (Spec Plan →
//! Execution Plan, ADR 0009). The #601 trigger conversion then resolves
//! the library by workflow id directly: no `spec_kind` string join, no
//! preset fallback.
//!
//! ## Why raw JSON, not the typed `WorkflowLibrary::register`
//!
//! Executor impls are typetag-registered in the **engine** (ADR 0012);
//! portal links only the substrate kernel, so it can neither construct
//! nor deserialize an `"agent"` executor as a Rust value — and must
//! not, per the two-process non-linking invariant (ADR 0027). Portal
//! authors the persisted wire form (`workflow_json`) directly; the
//! engine deserializes and kernel-validates it at load, which is the
//! existing contract for every library row. The envelope field names
//! are pinned by [`tests::envelope_matches_substrate_serde`] against
//! the substrate's own serde (via a `noop` workflow round-trip), and
//! the executor object mirrors the engine's `AgentExecutor` serde
//! (`model`, `system_prompt`; `tools` / `credential_ref` /
//! `runner` are `#[serde(default)]`-able or skipped).
//!
//! ## Executable subset (recorded on #602)
//!
//! Only `agent` (and `noop`) executors are registered in the engine
//! today; `external-check` / `manual-approval` stages have had no
//! runtime since the forge gate machine retired. They remain in the
//! authored definition (and the builder UI) but are excluded from the
//! derived DAG and surfaced `executable: false`; substrate executors
//! for them are a follow-up spec.

use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use crate::workflow::{GateKind, WorkflowStage};

/// Derive the executable DAG (as raw `workflow_json`) from the authored
/// stage list. Returns `None` when no stage is executable — the
/// workflow then has no library entry, and a fire fails loudly at the
/// engine's "no workflow registered" lookup rather than running an
/// empty plan.
pub fn derive_workflow_json(workflow_id: &str, stages: &[WorkflowStage]) -> Option<Value> {
    let agents: Vec<&WorkflowStage> = stages
        .iter()
        .filter(|s| s.gate_kind == GateKind::AgentSession)
        .collect();
    if agents.is_empty() {
        return None;
    }

    // Deterministic ids: derived from (workflow_id, position) so
    // re-registering an unchanged definition produces an identical
    // document (UUID v5 over a fixed namespace).
    let ns = Uuid::new_v5(&Uuid::NAMESPACE_OID, workflow_id.as_bytes());
    let edge_id = |i: usize| Uuid::new_v5(&ns, format!("edge:{i}").as_bytes()).to_string();
    let node_id = |i: usize| Uuid::new_v5(&ns, format!("node:{i}").as_bytes()).to_string();

    // A linear chain: edge 0 is the entry, edge i+1 carries node i's
    // output. Artifact ids are chain-local; the compiler namespaces
    // them per spec instance (invariant 5).
    let mut edges = Vec::with_capacity(agents.len() + 1);
    edges.push(json!({
        "id": edge_id(0),
        "artifact_id": format!("{workflow_id}:in"),
        "requires_deterministic": false,
    }));
    let mut nodes = Vec::with_capacity(agents.len());
    for (i, stage) in agents.iter().enumerate() {
        edges.push(json!({
            "id": edge_id(i + 1),
            "artifact_id": format!("{workflow_id}:out:{i}"),
            "requires_deterministic": false,
        }));
        let model = stage
            .params
            .get("model")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let system_prompt = stage
            .params
            .get("system_prompt")
            .or_else(|| stage.params.get("prompt"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        // Authored user-side task body (#616). `null` when absent —
        // the executor field is `Option` with serde(default).
        let user_prompt = stage.params.get("user_prompt").and_then(|v| v.as_str());
        nodes.push(json!({
            "id": node_id(i),
            "executor": {
                "kind": "agent",
                "model": model,
                "system_prompt": system_prompt,
                "user_prompt": user_prompt,
                "tools": [],
                "credential_ref": null,
            },
            "inputs": [{ "edge_id": edge_id(i) }],
            "outputs": [{ "edge_id": edge_id(i + 1) }],
        }));
    }

    Some(json!({
        "nodes": nodes,
        "edges": edges,
        "entry_specs": [{ "edge_id": edge_id(0) }],
        // Agent output is non-reproducible by definition — declared
        // provenance must match the exit path (kernel invariant 3).
        "output_specs": [{
            "edge_id": edge_id(agents.len()),
            "provenance": { "kind": "uncertain", "source": "agent" },
        }],
    }))
}

/// Register the derived DAG under `spec_kind = workflow_id`, bumping
/// the library version (same SQL contract as the substrate's
/// `WorkflowLibrary::register`). No-op when the definition has no
/// executable stage.
pub async fn register_executable(
    pool: &PgPool,
    workflow_id: &str,
    stages: &[WorkflowStage],
) -> anyhow::Result<Option<i32>> {
    let Some(workflow_json) = derive_workflow_json(workflow_id, stages) else {
        return Ok(None);
    };
    let row_id = Uuid::new_v4().to_string();
    let version: i32 = sqlx::query_scalar(
        "INSERT INTO workflow_library (id, spec_kind, version, workflow_json) \
         VALUES ($1, $2, \
                 COALESCE((SELECT MAX(version) FROM workflow_library WHERE spec_kind = $2), 0) + 1, \
                 $3) \
         RETURNING version",
    )
    .bind(&row_id)
    .bind(workflow_id)
    .bind(&workflow_json)
    .fetch_one(pool)
    .await?;
    Ok(Some(version))
}

/// Whether a stage's gate kind has a registered engine executor today.
pub fn stage_is_executable(kind: GateKind) -> bool {
    kind == GateKind::AgentSession
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stage(seq: i32, kind: GateKind, params: Value) -> WorkflowStage {
        WorkflowStage {
            id: format!("s{seq}"),
            workflow_id: "wf_test".into(),
            seq,
            gate_kind: kind,
            executable: stage_is_executable(kind),
            params,
        }
    }

    /// Pin the envelope field names against the substrate's own serde:
    /// a `noop` workflow built through the typed API must deserialize
    /// from a document with exactly the field layout this module
    /// authors. (The `agent` executor body itself can only deserialize
    /// in the engine, where the typetag impl is linked — the wave-close
    /// e2e covers that end.)
    #[test]
    fn envelope_matches_substrate_serde() {
        let mut doc = derive_workflow_json(
            "wf_env",
            &[stage(
                0,
                GateKind::AgentSession,
                json!({"model": "sonnet", "system_prompt": "x"}),
            )],
        )
        .expect("one agent stage derives");
        // Swap the engine-only executor for the substrate-linked noop;
        // everything else must round-trip through the typed Workflow.
        doc["nodes"][0]["executor"] = json!({ "kind": "noop" });
        let parsed: Result<onsager_substrate::Workflow, _> = serde_json::from_value(doc);
        let wf = parsed.expect("envelope deserializes against substrate serde");
        assert_eq!(wf.nodes.len(), 1);
        assert_eq!(wf.edges.len(), 2);
        assert_eq!(wf.entry_specs.len(), 1);
        assert_eq!(wf.output_specs.len(), 1);
    }

    /// #616: the stage's authored `user_prompt` param rides into the
    /// executor object (and `null`s cleanly when absent).
    #[test]
    fn user_prompt_param_reaches_executor() {
        let doc = derive_workflow_json(
            "wf_up",
            &[stage(
                0,
                GateKind::AgentSession,
                json!({"system_prompt": "s", "user_prompt": "do it"}),
            )],
        )
        .expect("derives");
        assert_eq!(doc["nodes"][0]["executor"]["user_prompt"], "do it");

        let doc = derive_workflow_json(
            "wf_up",
            &[stage(0, GateKind::AgentSession, json!({"system_prompt": "s"}))],
        )
        .expect("derives");
        assert!(doc["nodes"][0]["executor"]["user_prompt"].is_null());
    }

    /// The derivation is deterministic: same definition → identical
    /// document (re-saving an unchanged workflow re-registers the same
    /// content, just bumping the version).
    #[test]
    fn derivation_is_deterministic() {
        let stages = [stage(0, GateKind::AgentSession, json!({"model": "m"}))];
        assert_eq!(
            derive_workflow_json("wf_a", &stages),
            derive_workflow_json("wf_a", &stages)
        );
        assert_ne!(
            derive_workflow_json("wf_a", &stages),
            derive_workflow_json("wf_b", &stages)
        );
    }

    /// Chains chain: node i's output edge is node i+1's input edge.
    #[test]
    fn chain_edges_connect() {
        let stages = [
            stage(0, GateKind::AgentSession, json!({})),
            stage(1, GateKind::AgentSession, json!({})),
        ];
        let doc = derive_workflow_json("wf_chain", &stages).unwrap();
        let n0_out = &doc["nodes"][0]["outputs"][0]["edge_id"];
        let n1_in = &doc["nodes"][1]["inputs"][0]["edge_id"];
        assert_eq!(n0_out, n1_in);
        assert_eq!(doc["edges"].as_array().unwrap().len(), 3);
    }

    /// Non-agent stages are excluded from the executable DAG; a
    /// definition with no agent stage registers nothing.
    #[test]
    fn non_agent_stages_excluded() {
        let stages = [
            stage(0, GateKind::ExternalCheck, json!({})),
            stage(1, GateKind::AgentSession, json!({})),
            stage(2, GateKind::ManualApproval, json!({})),
        ];
        let doc = derive_workflow_json("wf_mixed", &stages).unwrap();
        assert_eq!(doc["nodes"].as_array().unwrap().len(), 1);
        assert!(
            derive_workflow_json("wf_none", &[stage(0, GateKind::ExternalCheck, json!({}))])
                .is_none()
        );
        assert!(stage_is_executable(GateKind::AgentSession));
        assert!(!stage_is_executable(GateKind::ExternalCheck));
    }
}
