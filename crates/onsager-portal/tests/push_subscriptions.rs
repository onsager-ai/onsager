//! Integration tests for the push-subscription storage layer (spec
//! #472).
//!
//! Skipped when `DATABASE_URL` is unset — matches the other portal
//! integration tests (chat_storage, etc.).

use onsager_portal::push_db;
use sqlx::PgPool;
use uuid::Uuid;

async fn try_pool() -> Option<PgPool> {
    let url = std::env::var("DATABASE_URL").ok()?;
    Some(PgPool::connect(&url).await.expect("portal connect"))
}

async fn cleanup_user(pool: &PgPool, user_id: &str) {
    let _ = sqlx::query("DELETE FROM push_subscriptions WHERE user_id = $1")
        .bind(user_id)
        .execute(pool)
        .await;
}

/// Round-trip: upsert a subscription, list it back.
#[tokio::test]
async fn upsert_and_list_round_trip() {
    let Some(pool) = try_pool().await else {
        eprintln!("skipping: DATABASE_URL not set");
        return;
    };
    let user_id = format!("u-{}", Uuid::new_v4());
    let workspace_id = format!("ws-{}", Uuid::new_v4());
    let endpoint = format!("https://push.example/{}", Uuid::new_v4());

    let row = push_db::upsert(
        &pool,
        &user_id,
        &workspace_id,
        &endpoint,
        "p256dh-key",
        "auth-secret",
        Some("Mozilla/5.0"),
    )
    .await
    .expect("upsert subscription");
    assert_eq!(row.user_id, user_id);
    assert_eq!(row.endpoint, endpoint);
    assert_eq!(row.p256dh_key, "p256dh-key");

    let subs = push_db::list_for_user(&pool, &user_id)
        .await
        .expect("list for user");
    assert_eq!(subs.len(), 1);
    assert_eq!(subs[0].id, row.id);

    cleanup_user(&pool, &user_id).await;
}

/// Re-subscribing the same endpoint upserts the row (same id, updated
/// keys) — the dispatch helper relies on this so subscription churn
/// doesn't fill the table with stale rows.
#[tokio::test]
async fn upsert_is_idempotent_on_endpoint() {
    let Some(pool) = try_pool().await else {
        eprintln!("skipping: DATABASE_URL not set");
        return;
    };
    let user_id = format!("u-{}", Uuid::new_v4());
    let endpoint = format!("https://push.example/{}", Uuid::new_v4());

    let first = push_db::upsert(&pool, &user_id, "ws-1", &endpoint, "k-old", "s-old", None)
        .await
        .unwrap();
    let second = push_db::upsert(
        &pool,
        &user_id,
        "ws-2",
        &endpoint,
        "k-new",
        "s-new",
        Some("UA"),
    )
    .await
    .unwrap();
    // Same row id; updated workspace + keys + user_agent.
    assert_eq!(first.id, second.id);
    assert_eq!(second.workspace_id, "ws-2");
    assert_eq!(second.p256dh_key, "k-new");
    assert_eq!(second.auth_secret, "s-new");
    assert_eq!(second.user_agent.as_deref(), Some("UA"));

    let subs = push_db::list_for_user(&pool, &user_id).await.unwrap();
    assert_eq!(subs.len(), 1);

    cleanup_user(&pool, &user_id).await;
}

/// Delete by endpoint is scoped to the caller — deleting user-A's
/// endpoint doesn't remove user-B's row with the same endpoint.
#[tokio::test]
async fn delete_by_endpoint_scoped_to_user() {
    let Some(pool) = try_pool().await else {
        eprintln!("skipping: DATABASE_URL not set");
        return;
    };
    let user_a = format!("u-{}", Uuid::new_v4());
    let user_b = format!("u-{}", Uuid::new_v4());
    let endpoint = format!("https://push.example/{}", Uuid::new_v4());

    push_db::upsert(&pool, &user_a, "ws-1", &endpoint, "k", "s", None)
        .await
        .unwrap();
    push_db::upsert(&pool, &user_b, "ws-1", &endpoint, "k", "s", None)
        .await
        .unwrap();

    let removed = push_db::delete_by_endpoint(&pool, &user_a, &endpoint)
        .await
        .unwrap();
    assert_eq!(removed, 1);

    let a_subs = push_db::list_for_user(&pool, &user_a).await.unwrap();
    let b_subs = push_db::list_for_user(&pool, &user_b).await.unwrap();
    assert!(a_subs.is_empty(), "user A should have no subs left");
    assert_eq!(b_subs.len(), 1, "user B's sub should be untouched");

    cleanup_user(&pool, &user_a).await;
    cleanup_user(&pool, &user_b).await;
}

/// `mark_used` stamps last_used_at — operator observability for dead
/// subscriptions.
#[tokio::test]
async fn mark_used_sets_last_used_at() {
    let Some(pool) = try_pool().await else {
        eprintln!("skipping: DATABASE_URL not set");
        return;
    };
    let user_id = format!("u-{}", Uuid::new_v4());
    let endpoint = format!("https://push.example/{}", Uuid::new_v4());

    let row = push_db::upsert(&pool, &user_id, "ws-1", &endpoint, "k", "s", None)
        .await
        .unwrap();
    assert!(row.last_used_at.is_none());

    push_db::mark_used(&pool, &row.id).await.unwrap();
    let subs = push_db::list_for_user(&pool, &user_id).await.unwrap();
    assert!(subs[0].last_used_at.is_some());

    cleanup_user(&pool, &user_id).await;
}
