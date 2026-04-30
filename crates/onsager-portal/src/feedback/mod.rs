//! Portal feedback contract — the "three feedback shapes" plumbing
//! (`#223`).
//!
//! Portal owns the public HTTP boundary. Behind that boundary, every
//! mutating request has one of three shapes:
//!
//! 1. **Read** (`GET`): SELECT against the spine, return.
//! 2. **Fast write** (`POST`/`PATCH`, `≤ 1s`): emit a spine intent
//!    stamped with a fresh `correlation_id`, await the matching response
//!    event, return synchronously.
//! 3. **Slow / streaming write**: same emit, return `202` with the
//!    `correlation_id`, dashboard subscribes for events tagged with it.
//!
//! This module is the type-system anchor for shapes 2 and 3. It owns:
//!
//! - [`dispatch`] — mint a `correlation_id`, write the intent through
//!   `EventStore`, return the handle.
//! - [`CorrelationRegistry`] — an in-process map of `correlation_id →
//!   oneshot::Sender<EventNotification>`. A background task tails the
//!   spine `pg_notify` channel and routes responses to the matching
//!   waiter without a DB roundtrip (the typed column added in spine
//!   migration 014 is mirrored on every notification).
//! - [`Waiter`] / [`await_with_timeout`] — bounded sync wait. Default
//!   `1s`, hard cap `2s` per the spec resolution comment on #223.
//! - [`command_response`] — emit-and-await convenience that falls
//!   through to the 202 path on timeout, so callers can write
//!   `match command_response(...).await { Sync(ev) => ..., Async(h) =>
//!   ... }` without juggling the registry directly.
//!
//! See <https://github.com/onsager-ai/onsager/issues/223> for the
//! full contract.

mod dispatch;
mod registry;

pub use dispatch::{
    command_response, dispatch, dispatch_with_id, CommandResponse, DispatchError, DispatchHandle,
};
pub use registry::{await_with_timeout, AwaitError, CorrelationRegistry, Waiter, MAX_SYNC_TIMEOUT};

/// Default timeout for fast-write helpers (`1s`). The hard cap is
/// [`MAX_SYNC_TIMEOUT`] (`2s`) — anything that needs more should be a
/// 202 + subscribe flow, not a longer await.
pub const DEFAULT_SYNC_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(1000);
