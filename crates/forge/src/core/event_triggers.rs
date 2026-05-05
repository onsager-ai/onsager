//! Event-trigger producers (#239).
//!
//! Three sub-adapters that all terminate by emitting `TriggerFired` to the
//! spine — same machinery as the GitHub-webhook flow, just with a different
//! source:
//!
//! - [`run_spine_event_listener`] — listens for `FactoryEventKind` events on
//!   the spine, matches them against active workflows whose trigger is
//!   [`TriggerKind::SpineEvent`], evaluates the optional `JsonFilter`, and
//!   re-emits `TriggerFired`.
//! - [`run_pg_notify_listener`] — runs one `LISTEN <channel>` per active
//!   [`TriggerKind::PgNotify`] workflow on a dedicated connection. Refreshes
//!   the channel set every 30s so a newly-created workflow starts firing
//!   without requiring a forge restart.
//! - [`run_outbox_poller`] — polls outbox tables every 2s
//!   (per #239 resolution), advancing a per-workflow cursor in the spine
//!   `outbox_trigger_cursor` sidecar.
//!
//! ## Loop guard
//!
//! A workflow with `SpineEvent { event_kind: "trigger.fired" }` would
//! self-amplify (every fire produces another `trigger.fired` event that the
//! same workflow listens to). [`SELF_REFERENTIAL_EVENT_KIND`] flags the
//! string; [`is_loop_amplifying_trigger`] is the runtime check. Stiglab's
//! `workflow_db.rs::insert_workflow_with_stages` rejects it at create time.

use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use async_trait::async_trait;
use chrono::Utc;
use serde_json::Value;
use sqlx::postgres::PgListener;
use sqlx::PgPool;
use tokio::sync::Mutex;
use tokio::time::interval;

use onsager_spine::factory_event::{FactoryEvent, FactoryEventKind};
use onsager_spine::{
    EventHandler, EventMetadata, EventNotification, EventStore, Listener, TriggerKind,
};

/// `event_kind` value that, if a `SpineEvent` workflow listened for it,
/// would self-amplify (its own emission re-fires the same workflow).
pub const SELF_REFERENTIAL_EVENT_KIND: &str = "trigger.fired";

/// Default outbox poll interval (per #239 resolution: 2s, fixed).
pub const OUTBOX_POLL_INTERVAL: Duration = Duration::from_secs(2);

/// Period to refresh the active set of `pg_notify` channels. New workflows
/// take at most this long to start firing.
pub const PG_NOTIFY_REFRESH_INTERVAL: Duration = Duration::from_secs(30);

/// Returns `true` when this trigger would create an amplification loop —
/// today only `SpineEvent { event_kind: "trigger.fired" }`. Used by both
/// the create-time check in stiglab's `workflow_db.rs` and the runtime
/// safeguard in [`run_spine_event_listener`].
pub fn is_loop_amplifying_trigger(trigger: &TriggerKind) -> bool {
    matches!(
        trigger,
        TriggerKind::SpineEvent { event_kind, .. } if event_kind == SELF_REFERENTIAL_EVENT_KIND
    )
}

// ---------------------------------------------------------------------------
// SpineEvent trigger listener
// ---------------------------------------------------------------------------

/// Run the spine-event trigger listener. Subscribes to all spine events,
/// matches each against active `SpineEvent` workflows, and emits
/// `TriggerFired` for matches.
pub async fn run_spine_event_listener(store: EventStore) -> anyhow::Result<()> {
    let dispatcher = SpineEventDispatcher {
        store: store.clone(),
    };
    Listener::new(store).run(dispatcher).await
}

struct SpineEventDispatcher {
    store: EventStore,
}

#[async_trait]
impl EventHandler for SpineEventDispatcher {
    async fn handle(&self, notification: EventNotification) -> anyhow::Result<()> {
        // Skip our own emissions to break the basic loop. The
        // create-time check rejects `SpineEvent { event_kind:
        // "trigger.fired" }` outright; this is the runtime backstop.
        if notification.event_type == SELF_REFERENTIAL_EVENT_KIND {
            return Ok(());
        }

        let payload = match load_event_payload(&self.store, &notification).await {
            Ok(Some(p)) => p,
            Ok(None) => return Ok(()),
            Err(e) => {
                tracing::warn!(
                    id = notification.id,
                    "forge event-trigger: load failed: {e}"
                );
                return Ok(());
            }
        };

        let candidates =
            match load_spine_event_candidates(self.store.pool(), &notification.event_type).await {
                Ok(c) => c,
                Err(e) => {
                    tracing::warn!(
                        event_type = %notification.event_type,
                        "forge event-trigger: candidate lookup failed: {e}"
                    );
                    return Ok(());
                }
            };

        for candidate in candidates {
            if is_loop_amplifying_trigger(&candidate.trigger) {
                continue;
            }
            let TriggerKind::SpineEvent { ref filter, .. } = candidate.trigger else {
                continue;
            };
            let matches = filter.as_ref().map(|f| f.matches(&payload)).unwrap_or(true);
            if !matches {
                continue;
            }
            if let Err(e) = emit_event_trigger_fired(
                &self.store,
                &candidate,
                "spine_event",
                serde_json::json!({
                    "source_event_id": notification.id,
                    "source_event_type": notification.event_type,
                    "source_payload": payload,
                }),
            )
            .await
            {
                tracing::warn!(
                    workflow_id = %candidate.workflow_id,
                    "forge event-trigger: emit failed: {e}"
                );
            }
        }
        Ok(())
    }
}

async fn load_event_payload(
    store: &EventStore,
    notification: &EventNotification,
) -> anyhow::Result<Option<Value>> {
    match notification.table.as_str() {
        "events" => match store.get_event_by_id(notification.id).await? {
            Some(row) => {
                // Strip the FactoryEvent wrapper if present.
                if let Ok(env) = serde_json::from_value::<FactoryEvent>(row.data.clone()) {
                    let kind_value = serde_json::to_value(env.event)?;
                    Ok(Some(kind_value))
                } else {
                    Ok(Some(row.data))
                }
            }
            None => Ok(None),
        },
        "events_ext" => match store.get_ext_event_by_id(notification.id).await? {
            Some(row) => {
                if let Ok(env) = serde_json::from_value::<FactoryEvent>(row.data.clone()) {
                    let kind_value = serde_json::to_value(env.event)?;
                    Ok(Some(kind_value))
                } else {
                    Ok(Some(row.data))
                }
            }
            None => Ok(None),
        },
        _ => Ok(None),
    }
}

// ---------------------------------------------------------------------------
// Common: load active workflows by trigger kind
// ---------------------------------------------------------------------------

/// One active workflow that's a candidate for an event-trigger fire.
#[derive(Debug, Clone)]
pub struct EventTriggerCandidate {
    pub workflow_id: String,
    pub workspace_id: String,
    pub trigger: TriggerKind,
}

async fn load_spine_event_candidates(
    pool: &PgPool,
    event_kind: &str,
) -> anyhow::Result<Vec<EventTriggerCandidate>> {
    let rows = sqlx::query(
        "SELECT workflow_id, workspace_id, trigger_kind, trigger_config \
           FROM workflows \
          WHERE active = TRUE \
            AND trigger_kind = 'spine_event' \
            AND trigger_config ->> 'event_kind' = $1",
    )
    .bind(event_kind)
    .fetch_all(pool)
    .await
    .context("loading spine_event candidates")?;
    Ok(rows.into_iter().filter_map(row_to_candidate).collect())
}

async fn load_pg_notify_candidates(pool: &PgPool) -> anyhow::Result<Vec<EventTriggerCandidate>> {
    let rows = sqlx::query(
        "SELECT workflow_id, workspace_id, trigger_kind, trigger_config \
           FROM workflows \
          WHERE active = TRUE \
            AND trigger_kind = 'pg_notify'",
    )
    .fetch_all(pool)
    .await
    .context("loading pg_notify candidates")?;
    Ok(rows.into_iter().filter_map(row_to_candidate).collect())
}

async fn load_outbox_candidates(pool: &PgPool) -> anyhow::Result<Vec<EventTriggerCandidate>> {
    let rows = sqlx::query(
        "SELECT workflow_id, workspace_id, trigger_kind, trigger_config \
           FROM workflows \
          WHERE active = TRUE \
            AND trigger_kind = 'outbox_row'",
    )
    .fetch_all(pool)
    .await
    .context("loading outbox_row candidates")?;
    Ok(rows.into_iter().filter_map(row_to_candidate).collect())
}

fn row_to_candidate(row: sqlx::postgres::PgRow) -> Option<EventTriggerCandidate> {
    use sqlx::Row;
    let workflow_id: String = row.try_get("workflow_id").ok()?;
    let workspace_id: String = row.try_get("workspace_id").ok()?;
    let kind_tag: String = row.try_get("trigger_kind").ok()?;
    let cfg: Value = row.try_get("trigger_config").ok()?;
    let trigger = TriggerKind::from_storage(&kind_tag, &cfg).ok()?;
    Some(EventTriggerCandidate {
        workflow_id,
        workspace_id,
        trigger,
    })
}

// ---------------------------------------------------------------------------
// Common: emit a trigger.fired payload for an event-driven fire
// ---------------------------------------------------------------------------

async fn emit_event_trigger_fired(
    store: &EventStore,
    candidate: &EventTriggerCandidate,
    source_kind: &str,
    extra: Value,
) -> anyhow::Result<()> {
    let now = Utc::now();
    let mut payload = serde_json::json!({
        "trigger_kind": candidate.trigger.kind_tag(),
        "workflow_id": candidate.workflow_id,
        "workspace_id": candidate.workspace_id,
        "fired_at": now,
        "source": source_kind,
    });
    if let Value::Object(ref mut map) = payload {
        if let Value::Object(extra_map) = extra {
            for (k, v) in extra_map {
                map.insert(k, v);
            }
        }
    }

    let envelope = FactoryEvent {
        event: FactoryEventKind::TriggerFired {
            workflow_id: candidate.workflow_id.clone(),
            trigger_kind: candidate.trigger.kind_tag().to_string(),
            payload,
        },
        correlation_id: None,
        causation_id: None,
        actor: format!("forge_{source_kind}"),
        timestamp: now,
    };
    let data = serde_json::to_value(&envelope)?;
    let metadata = EventMetadata {
        correlation_id: None,
        causation_id: None,
        actor: format!("forge_{source_kind}"),
    };
    store
        .append_ext(
            &candidate.workspace_id,
            &format!("workflow:{}", candidate.workflow_id),
            "workflow",
            "trigger.fired",
            data,
            &metadata,
            None,
        )
        .await
        .context("emitting trigger.fired")?;
    Ok(())
}

// ---------------------------------------------------------------------------
// PgNotify trigger listener
// ---------------------------------------------------------------------------

/// Cache of `PgNotify` workflows keyed by channel name. Refreshed on the
/// same cadence as the LISTEN set, then read directly in the recv path so a
/// burst of notifications doesn't trigger one DB lookup per message.
type PgNotifyCache = Arc<Mutex<std::collections::HashMap<String, Vec<EventTriggerCandidate>>>>;

/// Run the pg_notify trigger listener. Maintains one `LISTEN <channel>`
/// per active `PgNotify` workflow on a dedicated connection; refreshes the
/// channel set every [`PG_NOTIFY_REFRESH_INTERVAL`].
pub async fn run_pg_notify_listener(store: EventStore) -> anyhow::Result<()> {
    let mut listener = PgListener::connect_with(store.pool()).await?;
    let listened: Arc<Mutex<std::collections::HashSet<String>>> =
        Arc::new(Mutex::new(std::collections::HashSet::new()));
    let cache: PgNotifyCache = Arc::new(Mutex::new(std::collections::HashMap::new()));

    let mut refresh = interval(PG_NOTIFY_REFRESH_INTERVAL);

    refresh_pg_notify_channels(&mut listener, &store, &listened, &cache).await?;

    loop {
        tokio::select! {
            _ = refresh.tick() => {
                if let Err(e) = refresh_pg_notify_channels(
                    &mut listener,
                    &store,
                    &listened,
                    &cache,
                )
                .await
                {
                    tracing::warn!("forge pg_notify trigger: refresh failed: {e}");
                }
            }
            res = listener.recv() => {
                match res {
                    Ok(notif) => {
                        let payload_json: Value = serde_json::from_str(notif.payload())
                            .unwrap_or_else(|_| Value::String(notif.payload().to_string()));
                        if let Err(e) = handle_pg_notify(
                            &store,
                            &cache,
                            notif.channel(),
                            &payload_json,
                        ).await
                        {
                            tracing::warn!(
                                channel = %notif.channel(),
                                "forge pg_notify trigger: dispatch failed: {e}"
                            );
                        }
                    }
                    Err(e) => {
                        tracing::error!("forge pg_notify trigger: listener error: {e}");
                        return Err(e.into());
                    }
                }
            }
        }
    }
}

async fn refresh_pg_notify_channels(
    listener: &mut PgListener,
    store: &EventStore,
    listened: &Arc<Mutex<std::collections::HashSet<String>>>,
    cache: &PgNotifyCache,
) -> anyhow::Result<()> {
    let candidates = load_pg_notify_candidates(store.pool()).await?;
    let mut wanted: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut new_cache: std::collections::HashMap<String, Vec<EventTriggerCandidate>> =
        std::collections::HashMap::new();
    for c in &candidates {
        if let TriggerKind::PgNotify { channel, .. } = &c.trigger {
            if is_safe_channel_name(channel) {
                wanted.insert(channel.clone());
                new_cache
                    .entry(channel.clone())
                    .or_default()
                    .push(c.clone());
            } else {
                tracing::warn!(
                    workflow_id = %c.workflow_id,
                    channel = %channel,
                    "forge pg_notify trigger: skipping unsafe channel name"
                );
            }
        }
    }

    {
        let mut current = listened.lock().await;
        let to_listen: Vec<String> = wanted.difference(&current).cloned().collect();
        let to_unlisten: Vec<String> = current.difference(&wanted).cloned().collect();

        for ch in &to_listen {
            if let Err(e) = listener.listen(ch).await {
                tracing::warn!(channel = %ch, "forge pg_notify trigger: LISTEN failed: {e}");
            } else {
                current.insert(ch.clone());
            }
        }
        for ch in &to_unlisten {
            if let Err(e) = listener.unlisten(ch).await {
                tracing::warn!(channel = %ch, "forge pg_notify trigger: UNLISTEN failed: {e}");
            } else {
                current.remove(ch);
            }
        }
    }

    // Replace the cache atomically — recv-path lookups see a consistent
    // (channel → candidates) snapshot.
    *cache.lock().await = new_cache;
    Ok(())
}

/// Allow only conservative channel names — a-z, A-Z, 0-9, `_` — to keep
/// `LISTEN <channel>` from being a SQL injection surface. `PgListener::listen`
/// quotes the identifier, but we narrow further as defense in depth.
fn is_safe_channel_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 63
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

async fn handle_pg_notify(
    store: &EventStore,
    cache: &PgNotifyCache,
    channel: &str,
    payload: &Value,
) -> anyhow::Result<()> {
    // Read the channel → candidates map from cache instead of re-querying
    // for every NOTIFY. The cache is refreshed every
    // `PG_NOTIFY_REFRESH_INTERVAL`, so a freshly-created workflow takes at
    // most that long to start firing — same window as the LISTEN
    // discovery itself.
    let candidates: Vec<EventTriggerCandidate> = {
        let map = cache.lock().await;
        map.get(channel).cloned().unwrap_or_default()
    };
    for c in candidates {
        let TriggerKind::PgNotify { filter, .. } = &c.trigger else {
            continue;
        };
        let matches = filter.as_ref().map(|f| f.matches(payload)).unwrap_or(true);
        if !matches {
            continue;
        }
        emit_event_trigger_fired(
            store,
            &c,
            "pg_notify",
            serde_json::json!({ "channel": channel, "payload": payload }),
        )
        .await?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// OutboxRow trigger poller
// ---------------------------------------------------------------------------

/// Run the outbox-row trigger poller. Polls every
/// [`OUTBOX_POLL_INTERVAL`]; for each active `OutboxRow` workflow, queries
/// `<table>` for rows with `id > last_seen_id` matching `where_clause`,
/// emits `TriggerFired` for each, and advances the cursor in
/// `outbox_trigger_cursor`.
pub async fn run_outbox_poller(store: EventStore) -> anyhow::Result<()> {
    let mut tick = interval(OUTBOX_POLL_INTERVAL);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tick.tick().await;
        if let Err(e) = run_outbox_poll_once(&store).await {
            tracing::warn!("forge outbox poller: tick failed: {e}");
        }
    }
}

async fn run_outbox_poll_once(store: &EventStore) -> anyhow::Result<()> {
    let candidates = load_outbox_candidates(store.pool()).await?;
    for c in candidates {
        if let Err(e) = poll_outbox_for_workflow(store, &c).await {
            tracing::warn!(
                workflow_id = %c.workflow_id,
                "forge outbox poller: workflow poll failed: {e}"
            );
        }
    }
    Ok(())
}

async fn poll_outbox_for_workflow(
    store: &EventStore,
    candidate: &EventTriggerCandidate,
) -> anyhow::Result<()> {
    let TriggerKind::OutboxRow {
        table,
        where_clause,
    } = &candidate.trigger
    else {
        return Ok(());
    };

    if !is_safe_identifier(table) {
        anyhow::bail!("outbox table `{table}` is not a safe identifier");
    }
    if !is_safe_where_clause(where_clause) {
        anyhow::bail!("outbox where_clause contains unsafe characters");
    }

    let pool = store.pool();
    let cursor: i64 =
        sqlx::query_scalar("SELECT last_seen_id FROM outbox_trigger_cursor WHERE workflow_id = $1")
            .bind(&candidate.workflow_id)
            .fetch_optional(pool)
            .await?
            .unwrap_or(0);

    // Bounded batch — a long-stalled cursor with millions of rows shouldn't
    // produce a single multi-megabyte payload burst.
    let where_filter = if where_clause.trim().is_empty() {
        String::new()
    } else {
        format!("AND ({where_clause})")
    };
    let sql = format!(
        "SELECT id, row_to_json({table}.*) AS payload \
           FROM {table} \
          WHERE id > $1 {where_filter} \
          ORDER BY id ASC LIMIT 100"
    );
    let rows: Vec<(i64, Value)> = sqlx::query_as(&sql).bind(cursor).fetch_all(pool).await?;

    // Advance the cursor *before* emitting each row so a crash mid-batch
    // drops at most one fire instead of replaying the whole already-emitted
    // prefix on restart. Same reasoning as the scheduler's
    // persist-then-emit ordering.
    for (id, payload) in &rows {
        upsert_outbox_cursor(pool, &candidate.workflow_id, *id).await?;
        emit_event_trigger_fired(
            store,
            candidate,
            "outbox_row",
            serde_json::json!({
                "table": table,
                "row_id": id,
                "row": payload,
            }),
        )
        .await?;
    }
    Ok(())
}

async fn upsert_outbox_cursor(
    pool: &PgPool,
    workflow_id: &str,
    last_seen_id: i64,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO outbox_trigger_cursor (workflow_id, last_seen_id, last_seen_at) \
         VALUES ($1, $2, $3) \
         ON CONFLICT (workflow_id) DO UPDATE \
            SET last_seen_id = EXCLUDED.last_seen_id, \
                last_seen_at = EXCLUDED.last_seen_at",
    )
    .bind(workflow_id)
    .bind(last_seen_id)
    .bind(Utc::now())
    .execute(pool)
    .await?;
    Ok(())
}

/// Strict identifier check: matches the conservative subset Postgres
/// regular-identifier rules accept — ASCII letter / underscore start,
/// followed by ASCII letters / digits / underscores. Length capped at 63
/// (Postgres's `NAMEDATALEN-1`). Schema-qualified names are allowed:
/// `schema.table` is two identifiers separated by a single dot.
fn is_safe_identifier(s: &str) -> bool {
    if s.is_empty() || s.len() > 127 {
        return false;
    }
    s.split('.').all(|part| {
        let mut chars = part.chars();
        match chars.next() {
            Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
            _ => return false,
        }
        chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
    }) && s.len() <= 127
}

/// Defense-in-depth check on the user-supplied `where_clause`. Outbox
/// triggers are an admin-grade primitive (workflow authors who can create
/// `OutboxRow` workflows are trusted), but a real query parser is out of
/// scope for v1, so this guard catches the obvious DoS / injection shapes:
///
/// - statement terminators / comment markers (`;`, `--`, `/*`, `*/`)
/// - DoS-shaped function calls (`pg_sleep`, `pg_terminate_backend`,
///   `pg_cancel_backend`, `dblink`, `lo_*`)
/// - subquery / set-operator keywords (`select`, `union`, `intersect`,
///   `except`, `with`)
///
/// Matching is ASCII-case-insensitive and word-boundary-aware — the
/// substring `select` inside a column name like `selected` is allowed,
/// `SELECT 1` is not.
fn is_safe_where_clause(s: &str) -> bool {
    if s.contains(';') || s.contains("--") || s.contains("/*") || s.contains("*/") {
        return false;
    }
    const BANNED_KEYWORDS: &[&str] = &[
        "select",
        "union",
        "intersect",
        "except",
        "with",
        "pg_sleep",
        "pg_terminate_backend",
        "pg_cancel_backend",
        "dblink",
    ];
    let lower = s.to_ascii_lowercase();
    let bytes = lower.as_bytes();
    for kw in BANNED_KEYWORDS {
        let mut start = 0;
        while let Some(idx) = lower[start..].find(kw) {
            let absolute = start + idx;
            let prev_ok = absolute == 0 || !is_word_char(bytes[absolute - 1]);
            let after = absolute + kw.len();
            let next_ok = after >= bytes.len() || !is_word_char(bytes[after]);
            if prev_ok && next_ok {
                return false;
            }
            start = absolute + 1;
        }
    }
    true
}

fn is_word_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loop_amplifying_trigger_flags_self_referential_spine_event() {
        let bad = TriggerKind::SpineEvent {
            event_kind: "trigger.fired".into(),
            filter: None,
        };
        assert!(is_loop_amplifying_trigger(&bad));
    }

    #[test]
    fn loop_amplifying_trigger_passes_other_kinds() {
        let ok = TriggerKind::SpineEvent {
            event_kind: "forge.shaping_dispatched".into(),
            filter: None,
        };
        assert!(!is_loop_amplifying_trigger(&ok));

        let github = TriggerKind::GithubIssueWebhook {
            repo: "a/b".into(),
            label: "x".into(),
        };
        assert!(!is_loop_amplifying_trigger(&github));
    }

    #[test]
    fn safe_channel_names_pass() {
        assert!(is_safe_channel_name("factory_signal"));
        assert!(is_safe_channel_name("channel_42"));
        assert!(!is_safe_channel_name(""));
        assert!(!is_safe_channel_name("ch;DROP"));
        assert!(!is_safe_channel_name("ch with space"));
        assert!(!is_safe_channel_name(&"x".repeat(64)));
    }

    #[test]
    fn safe_identifier_rules() {
        assert!(is_safe_identifier("artifact_outbox"));
        assert!(is_safe_identifier("my_schema.outbox"));
        assert!(!is_safe_identifier(""));
        assert!(!is_safe_identifier("9_starts_with_digit"));
        assert!(!is_safe_identifier("table; DROP"));
        assert!(!is_safe_identifier("table--comment"));
    }

    #[test]
    fn safe_where_clause_rules() {
        // Allowed: simple comparisons, string literals, IS-NULL, allowed
        // word-suffixes that just happen to spell a banned keyword.
        assert!(is_safe_where_clause("status = 'sealed'"));
        assert!(is_safe_where_clause(""));
        assert!(is_safe_where_clause("flagged_for_selection IS NOT NULL"));
        // Statement terminators and comment markers.
        assert!(!is_safe_where_clause("1=1; DROP TABLE"));
        assert!(!is_safe_where_clause("x -- comment"));
        assert!(!is_safe_where_clause("/* nope */ 1=1"));
        // Subquery / set-operator keywords (case-insensitive, word-boundary).
        assert!(!is_safe_where_clause("id IN (SELECT id FROM other)"));
        assert!(!is_safe_where_clause("EXISTS(SELECT 1)"));
        assert!(!is_safe_where_clause("a UNION b"));
        assert!(!is_safe_where_clause("with x as (1) 1=1"));
        // DoS-shaped function calls.
        assert!(!is_safe_where_clause("pg_sleep(10) IS NULL"));
        assert!(!is_safe_where_clause("pg_terminate_backend(1) = TRUE"));
        assert!(!is_safe_where_clause("dblink('host', 'sql') IS NULL"));
    }
}
