//! Gate-request spine listener (spec #131 / ADR 0004 Lever C, phase 3).
//!
//! Tails `forge.gate_requested` events off the spine, deserializes the
//! payload into the typed [`FactoryEventKind`] variant, calls the
//! existing [`InterceptEngine`] (cached via [`EngineCache`]) to produce a
//! [`GateVerdict`], and emits the verdict back to the spine as
//! `synodic.gate_verdict` keyed on the same `gate_id` correlation handle.
//!
//! Replaces the legacy synchronous `forge → POST /api/gate` HTTP path
//! with an event-driven flow. The listener is the producer side of the
//! parking pattern: Forge parks the pipeline decision keyed by `gate_id`
//! when it emits `forge.gate_requested`, and the
//! `gate_verdict_listener` on Forge claims the parked decision when this
//! listener emits `synodic.gate_verdict`.
//!
//! ## Skipped events
//!
//! - Events written before phase 2 lacked the embedded `request` payload
//!   (`#[serde(default)]` decoded as `None`); we cannot evaluate without
//!   a request, so we skip and log. This only matters for replay of pre-
//!   phase-2 history; the steady-state contract is producer + this
//!   consumer always travel together.

use std::sync::Arc;

use async_trait::async_trait;
use onsager_spine::factory_event::FactoryEventKind;
use onsager_spine::protocol::{GateRequest, GateVerdict};
use onsager_spine::{EventHandler, EventMetadata, EventNotification, EventStore, Listener};

use crate::core::engine_cache::EngineCache;
use crate::core::gate_adapter;
use crate::core::storage::Storage;

/// Run the gate-request listener forever. Returns only if the underlying
/// pg_notify channel closes.
///
/// `since` is the backfill cursor — pass `EventStore::max_event_id()` so
/// a fresh boot doesn't replay the entire spine. Phase 6 will persist a
/// per-process cursor so a crash mid-decision doesn't drop in-flight
/// gate requests.
pub async fn run(
    store: EventStore,
    storage: Arc<dyn Storage>,
    engine_cache: Arc<EngineCache>,
    since: Option<i64>,
) -> anyhow::Result<()> {
    let dispatcher = Dispatcher {
        store: store.clone(),
        storage,
        engine_cache,
    };
    Listener::new(store).with_since(since).run(dispatcher).await
}

struct Dispatcher {
    store: EventStore,
    storage: Arc<dyn Storage>,
    engine_cache: Arc<EngineCache>,
}

impl Dispatcher {
    /// Pure: evaluate a [`GateRequest`] against the cached engine. Split
    /// out so the producer side can be tested without owning a spine.
    async fn evaluate(&self, request: &GateRequest) -> anyhow::Result<GateVerdict> {
        let engine = self.engine_cache.get_or_refresh(&*self.storage).await?;
        let intercept_req = gate_adapter::gate_request_to_intercept(request);
        let resp = engine.evaluate(&intercept_req);
        Ok(gate_adapter::intercept_to_gate_verdict(&resp))
    }

    async fn emit_verdict(
        &self,
        gate_id: &str,
        artifact_id: onsager_artifact::ArtifactId,
        gate_point: onsager_spine::factory_event::GatePoint,
        verdict: GateVerdict,
    ) -> anyhow::Result<()> {
        // #183: events_ext.workspace_id is a real column. Resolve from
        // the artifact this verdict is about; fall back to "default"
        // for the (rare) case where the artifact is no longer present
        // (e.g. archived between request and verdict). Lookup errors
        // are logged so a real DB problem doesn't silently mis-scope
        // every verdict (Copilot review on #235).
        let workspace_id = match self
            .store
            .lookup_workspace_for_artifact(artifact_id.as_str())
            .await
        {
            Ok(Some(ws)) => ws,
            Ok(None) => "default".to_string(),
            Err(e) => {
                tracing::warn!(
                    %artifact_id,
                    %gate_id,
                    "synodic workspace lookup failed; falling back to 'default': {e}"
                );
                "default".to_string()
            }
        };
        let event = FactoryEventKind::SynodicGateVerdict {
            gate_id: gate_id.to_string(),
            artifact_id,
            gate_point,
            verdict,
        };
        let data = serde_json::to_value(&event).expect("FactoryEventKind must serialize");
        let metadata = EventMetadata {
            actor: "synodic".into(),
            ..Default::default()
        };
        self.store
            .append_ext(
                &workspace_id,
                gate_id,
                "synodic",
                event.event_type(),
                data,
                &metadata,
                None,
            )
            .await?;
        Ok(())
    }
}

#[async_trait]
impl EventHandler for Dispatcher {
    async fn handle(&self, notification: EventNotification) -> anyhow::Result<()> {
        if notification.event_type != "forge.gate_requested" {
            return Ok(());
        }

        // Phase-3 producers always write into events_ext via append_ext.
        // The events table path is left unwired for now to avoid coupling
        // to a write site that doesn't exist; a future variant of the
        // emitter that writes there can extend this branch.
        if notification.table != "events_ext" {
            return Ok(());
        }

        let Some(row) = self.store.get_ext_event_by_id(notification.id).await? else {
            return Ok(());
        };

        let kind: FactoryEventKind = match serde_json::from_value(row.data.clone()) {
            Ok(k) => k,
            Err(e) => {
                tracing::warn!(
                    id = row.id,
                    "synodic: forge.gate_requested payload parse failed: {e}"
                );
                return Ok(());
            }
        };

        let FactoryEventKind::ForgeGateRequested {
            gate_id,
            artifact_id,
            gate_point,
            request,
        } = kind
        else {
            return Ok(());
        };

        let Some(request) = request else {
            // Pre-phase-2 events lack the embedded payload; nothing to
            // evaluate. Emitting a verdict without a request would be
            // policy-blind, so we skip and log.
            tracing::warn!(
                gate_id = %gate_id,
                artifact_id = %artifact_id,
                "synodic: forge.gate_requested has no embedded request payload, skipping"
            );
            return Ok(());
        };

        let verdict = self.evaluate(&request).await?;

        if let Err(e) = self
            .emit_verdict(&gate_id, artifact_id.clone(), gate_point, verdict)
            .await
        {
            tracing::error!(
                gate_id = %gate_id,
                artifact_id = %artifact_id,
                "synodic: failed to emit gate_verdict: {e}"
            );
        } else {
            tracing::info!(
                gate_id = %gate_id,
                artifact_id = %artifact_id,
                "synodic: emitted gate_verdict for forge.gate_requested"
            );
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    //! Listener-side tests focus on the typed-variant matching and
    //! request extraction. The evaluation pipeline (`gate_request_to_intercept`
    //! → engine → `intercept_to_gate_verdict`) is already covered by
    //! `gate_adapter::tests`; we test the listener glue separately.

    use onsager_artifact::ArtifactId;
    use onsager_spine::factory_event::{FactoryEventKind, GatePoint};

    /// Confirms that a [`FactoryEventKind::ForgeGateRequested`] without
    /// an embedded `request` is observably distinguishable from one with
    /// — the listener uses this branch to skip pre-phase-2 events
    /// instead of emitting a request-less verdict.
    #[test]
    fn forge_gate_requested_with_no_request_is_skip_signal() {
        let event = FactoryEventKind::ForgeGateRequested {
            gate_id: "g_legacy".into(),
            artifact_id: ArtifactId::new("art_x"),
            gate_point: GatePoint::PreDispatch,
            request: None,
        };
        match event {
            FactoryEventKind::ForgeGateRequested { request: None, .. } => {}
            other => panic!("expected request: None, got {other:?}"),
        }
    }
}
