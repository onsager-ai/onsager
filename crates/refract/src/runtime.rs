//! Refract runtime — ties the decomposer registry to the event spine.
//!
//! The runtime is the seam between "pure decomposition logic" (in
//! [`crate::decomposer`]) and "wire the result onto the spine" (here).
//! Tests can construct a runtime without a live spine by passing `None`
//! for the event store, in which case emissions become no-ops and results
//! are returned via the return value only.

use onsager_spine::factory_event::{FactoryEventKind, TokenUsage};
use onsager_spine::{EventMetadata, EventStore};

use crate::decomposer::{DecomposerError, DecomposerRegistry, DecompositionResult};
use crate::intent::Intent;

#[derive(thiserror::Error, Debug)]
pub enum RuntimeError {
    #[error("decomposer error: {0}")]
    Decomposer(#[from] DecomposerError),
    #[error("spine error: {0}")]
    Spine(String),
    #[error("serialize error: {0}")]
    Serialize(#[from] serde_json::Error),
}

/// The Refract runtime — holds a [`DecomposerRegistry`] and an optional
/// [`EventStore`] handle. When the spine handle is present, every intent
/// submission emits `refract.intent_submitted` and then either
/// `refract.decomposed` or `refract.failed`; when absent, only the
/// decomposer's return value is available.
pub struct Refract {
    registry: DecomposerRegistry,
    spine: Option<EventStore>,
}

impl Refract {
    pub fn new(registry: DecomposerRegistry, spine: Option<EventStore>) -> Self {
        Self { registry, spine }
    }

    pub fn registry(&self) -> &DecomposerRegistry {
        &self.registry
    }

    /// Submit an intent, emit `refract.intent_submitted`, run the matching
    /// decomposer, then emit either `refract.decomposed` (with the produced
    /// artifact ids) or `refract.failed` (with the error message).
    ///
    /// Returns the [`DecompositionResult`] on success so the caller can
    /// register the artifacts with Forge. Errors are recorded on the spine
    /// *and* returned to the caller — losing a decomposer failure in a
    /// fire-and-forget emission would make intent debugging opaque.
    pub async fn submit(&self, intent: &Intent) -> Result<DecompositionResult, RuntimeError> {
        self.emit_submitted(intent).await?;

        match self.registry.decompose(intent) {
            Ok(result) => {
                self.emit_decomposed(intent, &result).await?;
                Ok(result)
            }
            Err(err) => {
                let msg = err.to_string();
                self.emit_failed(intent, &msg).await?;
                Err(RuntimeError::Decomposer(err))
            }
        }
    }

    async fn emit_submitted(&self, intent: &Intent) -> Result<(), RuntimeError> {
        let event = FactoryEventKind::IntentSubmitted {
            intent_id: intent.id.to_string(),
            intent_class: intent.intent_class.clone(),
            description: intent.description.clone(),
            submitter: intent.submitter.clone(),
        };
        self.emit(&event).await
    }

    async fn emit_decomposed(
        &self,
        intent: &Intent,
        result: &DecompositionResult,
    ) -> Result<(), RuntimeError> {
        let event = FactoryEventKind::RefractDecomposed {
            intent_id: intent.id.to_string(),
            decomposer: intent.intent_class.clone(),
            artifact_ids: result
                .artifact_ids()
                .into_iter()
                .map(|a| a.to_string())
                .collect(),
        };
        self.emit(&event).await
    }

    async fn emit_failed(&self, intent: &Intent, reason: &str) -> Result<(), RuntimeError> {
        let event = FactoryEventKind::RefractFailed {
            intent_id: intent.id.to_string(),
            reason: reason.to_string(),
        };
        self.emit(&event).await
    }

    async fn emit(&self, event: &FactoryEventKind) -> Result<(), RuntimeError> {
        let Some(spine) = self.spine.as_ref() else {
            return Ok(());
        };
        let metadata = EventMetadata {
            actor: "refract".into(),
            ..Default::default()
        };
        let data = serde_json::to_value(event)?;
        let stream_id = event.stream_id();
        let event_type = event.event_type();
        spine
            .append_ext(&stream_id, "refract", event_type, data, &metadata, None)
            .await
            .map_err(|e| RuntimeError::Spine(e.to_string()))?;
        Ok(())
    }
}

/// Compile-time assertion: if a new [`FactoryEventKind`] variant is added
/// the `TokenUsage` import is still referenced — refract may want to
/// eventually surface LLM cost of LLM-backed decomposers (issue #39).
#[allow(dead_code)]
fn _compile_check(_u: TokenUsage) {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decomposer::FileMigrationDecomposer;
    use serde_json::json;

    fn registry() -> DecomposerRegistry {
        let mut r = DecomposerRegistry::new();
        r.register(FileMigrationDecomposer);
        r
    }

    #[tokio::test]
    async fn submit_without_spine_returns_result() {
        let r = Refract::new(registry(), None);
        let intent = Intent::new(
            FileMigrationDecomposer::NAME,
            "migrate",
            "marvin",
            json!({ "files": ["a.rs", "b.rs"] }),
        );
        let result = r.submit(&intent).await.expect("submit");
        assert_eq!(result.artifacts.len(), 2);
    }

    #[tokio::test]
    async fn submit_without_spine_surfaces_decomposer_error() {
        let r = Refract::new(registry(), None);
        let intent = Intent::new("nope", "unroutable", "marvin", json!({}));
        let err = r.submit(&intent).await.expect_err("unroutable");
        assert!(matches!(err, RuntimeError::Decomposer(_)));
    }
}
