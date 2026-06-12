//! `dispatch` — mint a `correlation_id` and append a spine intent.
//!
//! Portal-minted is the v1 contract (`#223` open-question resolution).
//! Subsystems propagate but never mint, so every spine intent has an
//! HTTP origin and a unique handle the portal knows how to await on.

use std::time::Duration;

use chrono::Utc;
use onsager_spine::{EventMetadata, EventNotification, EventStore, FactoryEvent, FactoryEventKind};
use uuid::Uuid;

use super::registry::{AwaitError, CorrelationRegistry};

/// Errors emitted by the dispatch helpers.
#[derive(Debug, thiserror::Error)]
pub enum DispatchError {
    #[error("spine append failed: {0}")]
    Append(#[from] sqlx::Error),
}

/// Handle returned from a dispatch — opaque to the caller, but
/// carries everything we need to either await synchronously or hand
/// the `correlation_id` back to the dashboard for a 202 + subscribe.
#[derive(Debug, Clone)]
pub struct DispatchHandle {
    pub correlation_id: Uuid,
    pub event_id: i64,
}

/// Mint a fresh `correlation_id`, wrap `event` in a [`FactoryEvent`]
/// envelope, and append it through `store`. Equivalent to
/// [`dispatch_with_id`] with a brand-new UUID.
pub async fn dispatch(
    store: &EventStore,
    event: FactoryEventKind,
    actor: impl Into<String>,
) -> Result<DispatchHandle, DispatchError> {
    dispatch_with_id(store, event, actor, Uuid::new_v4()).await
}

/// Like [`dispatch`] but with a caller-supplied `correlation_id` —
/// used by [`command_response`] (which has to register the waiter
/// **before** dispatching to avoid a TOCTOU between the spine
/// notification and the registry insert).
pub async fn dispatch_with_id(
    store: &EventStore,
    event: FactoryEventKind,
    actor: impl Into<String>,
    correlation_id: Uuid,
) -> Result<DispatchHandle, DispatchError> {
    let actor = actor.into();
    let envelope = FactoryEvent {
        event,
        correlation_id: Some(correlation_id.to_string()),
        causation_id: None,
        actor: actor.clone(),
        timestamp: Utc::now(),
    };
    let metadata = EventMetadata {
        correlation_id: Some(correlation_id.to_string()),
        causation_id: None,
        actor,
    };
    let event_id = store.append_factory_event(&envelope, &metadata).await?;
    Ok(DispatchHandle {
        correlation_id,
        event_id,
    })
}

/// Outcome of [`command_response`]: either the matching response
/// arrived inside the timeout (sync), or it didn't and the caller
/// should hand the dispatch handle to the dashboard for a 202 +
/// subscribe (async).
#[derive(Debug)]
pub enum CommandResponse {
    Synchronous(EventNotification),
    Async(DispatchHandle),
}

/// Emit `event`, then wait up to `timeout` for a response event
/// stamped with the same `correlation_id`. On timeout, returns
/// `Async(handle)` so the HTTP handler can fall through to a 202
/// without raising a user-visible error.
///
/// `timeout` is clamped to [`super::MAX_SYNC_TIMEOUT`] (`2s`); pass
/// [`super::DEFAULT_SYNC_TIMEOUT`] if you have no specific budget.
pub async fn command_response(
    registry: &CorrelationRegistry,
    store: &EventStore,
    event: FactoryEventKind,
    actor: impl Into<String>,
    timeout: Duration,
) -> Result<CommandResponse, DispatchError> {
    let correlation_id = Uuid::new_v4();
    // Register the waiter BEFORE dispatch so we never miss a
    // response that lands faster than the registry insert.
    let waiter = registry.register(correlation_id);
    let handle = dispatch_with_id(store, event, actor, correlation_id).await?;
    match waiter.timeout(timeout).await {
        Ok(notification) => Ok(CommandResponse::Synchronous(notification)),
        Err(AwaitError::Timeout) => Ok(CommandResponse::Async(handle)),
        Err(AwaitError::Cancelled) => Ok(CommandResponse::Async(handle)),
    }
}
