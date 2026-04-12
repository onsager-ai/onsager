//! High-level subscription API on top of [`EventStore::subscribe`].
//!
//! # Namespace filtering convention (v0.1)
//!
//! `pg_notify` notifications carry `stream_id` but **not** the namespace
//! column. As a v0.1 contract, producers are expected to prefix their
//! `stream_id` values with the namespace followed by a colon — e.g.
//! `"stiglab:session:abc"`. The [`Listener`] filters incoming notifications by
//! splitting `stream_id` on the first `':'` and comparing the prefix against
//! its subscribed namespaces. If the prefix does not match any subscribed
//! namespace the notification is dropped.
//!
//! If [`Listener::subscribe`] is never called, the listener forwards
//! **all** notifications (no filtering).

use std::collections::HashSet;
use std::sync::Arc;

use async_trait::async_trait;

use crate::namespace::Namespace;
use crate::store::{EventNotification, EventStore};

/// Trait implemented by consumers that want to react to events.
#[async_trait]
pub trait EventHandler: Send + Sync + 'static {
    /// Handle a single event notification. Returning an error logs the failure
    /// but does **not** stop the listener.
    async fn handle(&self, event: EventNotification) -> anyhow::Result<()>;
}

/// A high-level event listener that filters notifications by namespace and
/// dispatches them to an [`EventHandler`].
pub struct Listener {
    store: EventStore,
    namespaces: HashSet<String>,
}

impl Listener {
    /// Create a new listener backed by the given event store.
    pub fn new(store: EventStore) -> Self {
        Self {
            store,
            namespaces: HashSet::new(),
        }
    }

    /// Subscribe to events from the given namespace. Chainable.
    ///
    /// If never called, the listener forwards all notifications.
    pub fn subscribe(mut self, ns: Namespace) -> Self {
        self.namespaces.insert(ns.as_str().to_owned());
        self
    }

    /// Run the listener loop. This is long-running and only returns when the
    /// underlying `pg_notify` channel closes.
    ///
    /// Each notification is dispatched to `handler` in its own Tokio task so
    /// that a slow handler does not block the channel.
    pub async fn run<H: EventHandler>(self, handler: H) -> anyhow::Result<()> {
        let mut rx = self.store.subscribe().await?;
        let handler = Arc::new(handler);

        while let Some(notification) = rx.recv().await {
            if !self.namespaces.is_empty()
                && !matches_any_namespace(&notification.stream_id, &self.namespaces)
            {
                continue;
            }

            let handler = Arc::clone(&handler);
            tokio::spawn(async move {
                if let Err(e) = handler.handle(notification).await {
                    tracing::error!("EventHandler error: {e}");
                }
            });
        }

        tracing::warn!("pg_notify channel closed, listener shutting down");
        Ok(())
    }
}

/// Check whether `stream_id` starts with any of the given namespace prefixes.
///
/// The convention is `"<namespace>:<rest>"` — we split on the first `':'`.
fn matches_any_namespace(stream_id: &str, namespaces: &HashSet<String>) -> bool {
    match stream_id.split_once(':') {
        Some((prefix, _)) => namespaces.contains(prefix),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // We test the pure matching logic directly rather than constructing a real
    // Listener, which would require a live PgPool.

    fn ns_set(names: &[&str]) -> HashSet<String> {
        names.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn matches_single_namespace() {
        let namespaces = ns_set(&["stiglab"]);
        assert!(matches_any_namespace("stiglab:session:abc", &namespaces));
        assert!(!matches_any_namespace("synodic:session:abc", &namespaces));
    }

    #[test]
    fn matches_multiple_namespaces() {
        let namespaces = ns_set(&["stiglab", "ising"]);
        assert!(matches_any_namespace("stiglab:session:1", &namespaces));
        assert!(matches_any_namespace("ising:run:42", &namespaces));
        assert!(!matches_any_namespace("synodic:policy:x", &namespaces));
    }

    #[test]
    fn no_colon_never_matches() {
        let namespaces = ns_set(&["stiglab"]);
        assert!(!matches_any_namespace("stiglab", &namespaces));
        assert!(!matches_any_namespace("no-colon-here", &namespaces));
    }

    #[test]
    fn empty_namespace_set_is_handled_by_caller() {
        // When the namespace set is empty, the Listener skips filtering
        // entirely. This test just documents that matches_any_namespace returns
        // false for an empty set — the caller decides what that means.
        let namespaces = ns_set(&[]);
        assert!(!matches_any_namespace("stiglab:session:1", &namespaces));
    }

    #[test]
    fn prefix_only_up_to_first_colon() {
        let namespaces = ns_set(&["stiglab"]);
        // "stiglab:session:abc" — prefix is "stiglab", not "stiglab:session"
        assert!(matches_any_namespace("stiglab:session:abc", &namespaces));
        // Make sure a namespace with colon in it doesn't partially match
        let namespaces = ns_set(&["stiglab:session"]);
        assert!(!matches_any_namespace("stiglab:session:abc", &namespaces));
    }
}
