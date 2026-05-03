//! `ising serve` — start the Ising observation loop (issue #36 MVP).
//!
//! Each tick:
//!   1. Query recent forge events from `events_ext` (artifact.registered +
//!      forge.gate_verdict) to rebuild a fresh [`FactoryModel`].
//!   2. Run the registered analyzers against the model.
//!   3. Pass each produced insight through the [`InsightEmitter`]
//!      (validation + dedup) and append accepted emissions to `events_ext`
//!      as `ising.insight_emitted` so Forge and the dashboard can read
//!      them.
//!
//! The rebuild-per-tick approach is intentionally naive — it's O(N) in
//! recent events but avoids a second cursor source-of-truth while the
//! ising pipeline is bedding in. A streaming `Listener`-based path is the
//! next iteration once the emission contract is stable.

use std::collections::HashSet;

use chrono::{Duration, Utc};
use onsager_artifact::{ArtifactId, Kind};
use onsager_spine::factory_event::{FactoryEventKind, GatePoint, ShapingOutcome, VerdictSummary};
use onsager_spine::{EventMetadata, EventStore};

use crate::analyzers::register_defaults;
use crate::core::emitter::{EmitResult, EmitterConfig, InsightEmitter};
use crate::core::{
    insight_to_emitted_event, insight_to_rule_proposal, AnalyzerRegistry, FactoryModel,
};

/// How far back to look for forge events when rebuilding the factory model.
/// Matches the default `GateOverrideConfig::window` so insights have enough
/// evidence to compute stable override ratios.
const LOOKBACK: Duration = Duration::days(7);

/// Upper bound on how many recent events to pull per tick. Comfortably above
/// the expected volume for a dev factory; real deployments will want a
/// cursor-based stream instead.
const EVENT_FETCH_LIMIT: i64 = 2000;

/// Start the Ising observation loop.
pub fn run(database_url: &str, tick_ms: u64) {
    let rt = tokio::runtime::Runtime::new().expect("failed to create tokio runtime");
    rt.block_on(async move {
        tracing_subscriber::fmt()
            .with_env_filter("ising=info")
            .init();

        tracing::info!(tick_ms, "ising: starting observation loop");

        let spine = match EventStore::connect(database_url).await {
            Ok(s) => {
                tracing::info!("ising: connected to event spine");
                s
            }
            Err(e) => {
                tracing::error!("ising: spine connection failed ({e}); exiting");
                return;
            }
        };

        let mut registry = AnalyzerRegistry::new();
        register_defaults(&mut registry);
        tracing::info!(analyzers = registry.len(), "ising: analyzer registry ready");

        // Long-lived emitter: keeping the dedup window across ticks prevents
        // the same insight pattern from being re-appended to `events_ext` on
        // every rebuild as long as its fingerprint is unchanged. Without
        // this the spine — and the dashboard reading from it — would get
        // flooded with duplicates every `tick_ms` while the evidence
        // remains inside the lookback window.
        let mut emitter = InsightEmitter::new(EmitterConfig::default());

        let mut interval = tokio::time::interval(std::time::Duration::from_millis(tick_ms));
        loop {
            interval.tick().await;
            if let Err(e) = run_tick(&spine, &registry, &mut emitter).await {
                tracing::error!("ising: tick failed: {e}");
            }
        }
    });
}

async fn run_tick(
    spine: &EventStore,
    registry: &AnalyzerRegistry,
    emitter: &mut InsightEmitter,
) -> Result<(), anyhow::Error> {
    let model = build_model(spine).await?;
    if model.events_processed == 0 {
        // No observable factory activity yet — nothing to reason about.
        return Ok(());
    }

    let mut emitted = 0usize;
    let mut proposals = 0usize;
    for (analyzer_name, insights) in registry.run_all(&model) {
        for insight in insights {
            match emitter.emit(insight) {
                EmitResult::Accepted { insight, .. } => {
                    let event = insight_to_emitted_event(&analyzer_name, &insight);
                    if let Err(e) = append_insight_emitted(spine, &event).await {
                        tracing::warn!("ising: failed to append insight event: {e}");
                        continue;
                    }
                    emitted += 1;

                    // Issue #36 Step 2: for signals that warrant a rule
                    // change, emit `ising.rule_proposed` alongside the
                    // observation event. The proposal is paired with its
                    // backing `insight_emitted` via `insight_id` so Synodic
                    // can audit the evidence without a second query.
                    if let Some(proposal) = insight_to_rule_proposal(&analyzer_name, &insight) {
                        if let Err(e) = append_rule_proposed(spine, &proposal).await {
                            tracing::warn!("ising: failed to append rule_proposed event: {e}");
                        } else {
                            proposals += 1;
                        }
                    }
                }
                EmitResult::Suppressed { reason, .. } => {
                    tracing::debug!(
                        analyzer = analyzer_name,
                        reason,
                        "ising: insight suppressed"
                    );
                }
                EmitResult::Rejected { reason } => {
                    tracing::warn!(analyzer = analyzer_name, reason, "ising: insight rejected");
                }
            }
        }
    }

    if emitted > 0 || proposals > 0 {
        tracing::info!(emitted, proposals, "ising: tick emitted insights");
    }
    Ok(())
}

/// Rebuild an in-memory [`FactoryModel`] from the recent-events window in
/// `events_ext`. Parses the hand-coded JSON payloads Forge / Stiglab / the
/// portal write today — once events are emitted through a typed spine helper
/// this parser collapses to `serde_json::from_value`.
async fn build_model(spine: &EventStore) -> Result<FactoryModel, anyhow::Error> {
    let cutoff = Utc::now() - LOOKBACK;
    // Pull both the `forge` and `git` namespaces — Phase 3 analyzers
    // (`pr_churn`, `gate_deny_rate`) need the full PR + verdict stream.
    let mut rows = spine
        .query_ext_events(None, Some("forge"), EVENT_FETCH_LIMIT)
        .await?;
    let git_rows = spine
        .query_ext_events(None, Some("git"), EVENT_FETCH_LIMIT)
        .await?;
    rows.extend(git_rows);

    // If we pulled the whole fetch cap on either namespace and the oldest
    // row we got is still inside the lookback window, there are events we
    // didn't load. The override rate is then computed from a truncated
    // slice — surface it loudly rather than silently under-count.
    if rows.len() as i64 >= EVENT_FETCH_LIMIT * 2
        && rows
            .iter()
            .map(|r| r.created_at)
            .min()
            .is_some_and(|oldest| oldest >= cutoff)
    {
        tracing::warn!(
            event_fetch_limit = EVENT_FETCH_LIMIT,
            lookback_days = LOOKBACK.num_days(),
            "ising: spine fetch cap reached before lookback cutoff; \
             model window may be incomplete"
        );
    }

    // Rows come back newest-first; ingest oldest-first so event_id ordering
    // inside the model matches spine order.
    rows.sort_by_key(|r| r.id);

    let mut model = FactoryModel::new();
    let mut seen_ids: HashSet<i64> = HashSet::new();

    for row in rows {
        if !seen_ids.insert(row.id) {
            continue;
        }
        if row.created_at < cutoff {
            continue;
        }
        let Some(event) = parse_forge_event(&row.event_type, &row.data) else {
            continue;
        };
        // Use the spine row's `created_at` so windowed analyzers honor event
        // time, not ingest time — otherwise every tick's rebuild makes old
        // events look fresh.
        model.ingest_at(row.id, row.created_at, &event);
    }

    Ok(model)
}

/// Translate one forge-namespace row from `events_ext` into a
/// [`FactoryEventKind`] the model can ingest. Returns `None` for event types
/// Ising doesn't care about, or payloads too malformed to use.
fn parse_forge_event(event_type: &str, data: &serde_json::Value) -> Option<FactoryEventKind> {
    match event_type {
        "artifact.registered" => {
            let artifact_id = data.get("artifact_id")?.as_str()?;
            let kind_str = data.get("kind")?.as_str()?;
            let name = data
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let owner = data
                .get("owner")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            Some(FactoryEventKind::ArtifactRegistered {
                artifact_id: ArtifactId::new(artifact_id),
                kind: parse_kind(kind_str),
                name,
                owner,
            })
        }
        "forge.gate_verdict" => {
            let artifact_id = data.get("artifact_id")?.as_str()?;
            let gate_point = parse_gate_point(data.get("gate_point")?.as_str()?)?;
            let verdict = parse_verdict(data.get("verdict")?.as_str()?)?;
            Some(FactoryEventKind::ForgeGateVerdict {
                artifact_id: ArtifactId::new(artifact_id),
                gate_point,
                verdict,
            })
        }
        // `git.pr_*` events arrive from `onsager-portal` (issue #60). Phase
        // 3 analyzers (`pr_churn`, `gate_deny_rate`) are the consumers; the
        // model already knows how to ingest these variants.
        "git.pr_opened" => {
            let artifact_id = data.get("artifact_id")?.as_str()?;
            let pr_number = data.get("pr_number")?.as_u64()?;
            let repo = data
                .get("repo")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let url = data
                .get("url")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            Some(FactoryEventKind::GitPrOpened {
                artifact_id: ArtifactId::new(artifact_id),
                repo,
                pr_number,
                url,
            })
        }
        "git.pr_merged" => {
            let artifact_id = data.get("artifact_id")?.as_str()?;
            let pr_number = data.get("pr_number")?.as_u64()?;
            let merge_sha = data
                .get("merge_sha")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            Some(FactoryEventKind::GitPrMerged {
                artifact_id: ArtifactId::new(artifact_id),
                pr_number,
                merge_sha,
            })
        }
        "forge.shaping_returned" => {
            // Required for the `shape_retry_spike` analyzer — without ingest
            // here, `FactoryModel.shaping_records` stays empty and the
            // analyzer never fires in production.
            let request_id = data.get("request_id")?.as_str()?.to_string();
            let artifact_id = data.get("artifact_id")?.as_str()?;
            let outcome = parse_shaping_outcome(data.get("outcome")?.as_str()?)?;
            Some(FactoryEventKind::ForgeShapingReturned {
                request_id,
                artifact_id: ArtifactId::new(artifact_id),
                outcome,
            })
        }
        _ => None,
    }
}

fn parse_kind(s: &str) -> Kind {
    match s {
        "code" => Kind::Code,
        "document" => Kind::Document,
        "pull_request" => Kind::PullRequest,
        other => Kind::Custom(other.to_string()),
    }
}

fn parse_gate_point(s: &str) -> Option<GatePoint> {
    // Forge emits via `{:?}` (Debug), so strings arrive as CamelCase variant
    // names; Synodic may emit the serde snake_case form. Accept both.
    match s {
        "PreDispatch" | "pre_dispatch" => Some(GatePoint::PreDispatch),
        "StateTransition" | "state_transition" => Some(GatePoint::StateTransition),
        "ConsumerRouting" | "consumer_routing" => Some(GatePoint::ConsumerRouting),
        "ToolLevel" | "tool_level" => Some(GatePoint::ToolLevel),
        _ => None,
    }
}

fn parse_verdict(s: &str) -> Option<VerdictSummary> {
    match s {
        "Allow" | "allow" => Some(VerdictSummary::Allow),
        "Deny" | "deny" => Some(VerdictSummary::Deny),
        "Modify" | "modify" => Some(VerdictSummary::Modify),
        "Escalate" | "escalate" => Some(VerdictSummary::Escalate),
        _ => None,
    }
}

fn parse_shaping_outcome(s: &str) -> Option<ShapingOutcome> {
    // Forge emits via `format!("{:?}", outcome)` (Debug), so strings arrive
    // as CamelCase. A future typed emitter would use serde snake_case —
    // accept both, like `parse_gate_point` / `parse_verdict`.
    match s {
        "Completed" | "completed" => Some(ShapingOutcome::Completed),
        "Failed" | "failed" => Some(ShapingOutcome::Failed),
        "Partial" | "partial" => Some(ShapingOutcome::Partial),
        "Aborted" | "aborted" => Some(ShapingOutcome::Aborted),
        _ => None,
    }
}

/// Append an `ising.insight_emitted` row to `events_ext` so Forge and the
/// dashboard can consume it. Uses the same pattern as Forge's hand-coded
/// emissions so dashboard queries (`namespace = "ising"`) keep working.
async fn append_insight_emitted(
    spine: &EventStore,
    event: &FactoryEventKind,
) -> Result<i64, anyhow::Error> {
    let FactoryEventKind::IsingInsightEmitted {
        signal_kind,
        subject_ref,
        evidence,
        confidence,
    } = event
    else {
        return Err(anyhow::anyhow!(
            "append_insight_emitted called with non-IsingInsightEmitted variant"
        ));
    };

    let stream_id = format!("ising:{subject_ref}");
    let data = serde_json::json!({
        "signal_kind": signal_kind,
        "subject_ref": subject_ref,
        "evidence": evidence,
        "confidence": confidence,
    });
    let meta = EventMetadata {
        actor: "ising".to_string(),
        ..Default::default()
    };

    // #183: events_ext.workspace_id is a real column. `subject_ref`
    // is free-form (artifact id, artifact kind, rule id) — try
    // lookup_workspace_for_artifact and fall back to "default" for
    // non-artifact subjects.
    let workspace_id = spine
        .lookup_workspace_for_artifact(subject_ref)
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| "default".to_string());

    let id = spine
        .append_ext(
            &workspace_id,
            &stream_id,
            "ising",
            "ising.insight_emitted",
            data,
            &meta,
            None,
        )
        .await?;
    Ok(id)
}

/// Append an `ising.rule_proposed` row to `events_ext` so Synodic's
/// proposal-queue listener can pick it up (issue #36 Step 2). The payload
/// is a full serialization of the `IsingRuleProposed` variant so the
/// consumer can `serde_json::from_value::<FactoryEventKind>` without a
/// second parser, unlike the hand-rolled `insight_emitted` body.
async fn append_rule_proposed(
    spine: &EventStore,
    event: &FactoryEventKind,
) -> Result<i64, anyhow::Error> {
    let FactoryEventKind::IsingRuleProposed { subject_ref, .. } = event else {
        return Err(anyhow::anyhow!(
            "append_rule_proposed called with non-IsingRuleProposed variant"
        ));
    };

    let stream_id = event.stream_id();
    let data = serde_json::to_value(event)?;
    let meta = EventMetadata {
        actor: "ising".to_string(),
        ..Default::default()
    };

    // #183: same pattern as insight_emitted — `subject_ref` may name
    // an artifact, an artifact kind, or a rule id. Try the artifact
    // lookup and fall back to "default" for non-artifact subjects.
    let workspace_id = spine
        .lookup_workspace_for_artifact(subject_ref)
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| "default".to_string());

    let id = spine
        .append_ext(
            &workspace_id,
            &stream_id,
            "ising",
            "ising.rule_proposed",
            data,
            &meta,
            None,
        )
        .await?;
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_artifact_registered_payload() {
        let data = json!({
            "artifact_id": "art_x",
            "kind": "code",
            "name": "svc",
            "owner": "marvin",
        });
        let parsed = parse_forge_event("artifact.registered", &data).expect("parses");
        match parsed {
            FactoryEventKind::ArtifactRegistered {
                artifact_id, kind, ..
            } => {
                assert_eq!(artifact_id.as_str(), "art_x");
                assert_eq!(kind, Kind::Code);
            }
            _ => panic!(),
        }
    }

    #[test]
    fn parses_gate_verdict_debug_format() {
        // Forge emits via `{:?}`: CamelCase variants.
        let data = json!({
            "artifact_id": "art_x",
            "gate_point": "PreDispatch",
            "verdict": "Deny",
        });
        let parsed = parse_forge_event("forge.gate_verdict", &data).expect("parses");
        match parsed {
            FactoryEventKind::ForgeGateVerdict {
                gate_point,
                verdict,
                ..
            } => {
                assert_eq!(gate_point, GatePoint::PreDispatch);
                assert_eq!(verdict, VerdictSummary::Deny);
            }
            _ => panic!(),
        }
    }

    #[test]
    fn parses_gate_verdict_snake_case_format() {
        // A future typed emitter would use serde snake_case — accept that too.
        let data = json!({
            "artifact_id": "art_x",
            "gate_point": "state_transition",
            "verdict": "escalate",
        });
        let parsed = parse_forge_event("forge.gate_verdict", &data).expect("parses");
        match parsed {
            FactoryEventKind::ForgeGateVerdict {
                gate_point,
                verdict,
                ..
            } => {
                assert_eq!(gate_point, GatePoint::StateTransition);
                assert_eq!(verdict, VerdictSummary::Escalate);
            }
            _ => panic!(),
        }
    }

    #[test]
    fn ignores_unknown_event_types() {
        let data = json!({});
        assert!(parse_forge_event("forge.unknown", &data).is_none());
    }

    #[test]
    fn parses_shaping_returned_debug_format() {
        // Forge emits the outcome via `format!("{:?}", outcome)` — pin the
        // CamelCase parse so the `shape_retry_spike` analyzer actually sees
        // shaping records in production.
        let data = json!({
            "request_id": "req_x",
            "artifact_id": "art_x",
            "outcome": "Completed",
        });
        let parsed = parse_forge_event("forge.shaping_returned", &data).expect("parses");
        match parsed {
            FactoryEventKind::ForgeShapingReturned {
                request_id,
                artifact_id,
                outcome,
            } => {
                assert_eq!(request_id, "req_x");
                assert_eq!(artifact_id.as_str(), "art_x");
                assert_eq!(outcome, ShapingOutcome::Completed);
            }
            _ => panic!("expected ForgeShapingReturned"),
        }
    }

    #[test]
    fn parses_shaping_returned_snake_case_format() {
        // A future typed emitter would use serde snake_case — accept that
        // too, mirroring `parse_gate_verdict_snake_case_format`.
        let data = json!({
            "request_id": "req_y",
            "artifact_id": "art_y",
            "outcome": "failed",
        });
        let parsed = parse_forge_event("forge.shaping_returned", &data).expect("parses");
        match parsed {
            FactoryEventKind::ForgeShapingReturned { outcome, .. } => {
                assert_eq!(outcome, ShapingOutcome::Failed);
            }
            _ => panic!("expected ForgeShapingReturned"),
        }
    }

    #[test]
    fn shaping_returned_with_unknown_outcome_returns_none() {
        let data = json!({
            "request_id": "req_z",
            "artifact_id": "art_z",
            "outcome": "WeirdNewVariant",
        });
        assert!(parse_forge_event("forge.shaping_returned", &data).is_none());
    }
}
