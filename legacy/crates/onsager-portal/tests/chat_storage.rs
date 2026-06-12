//! Integration tests for the chat storage backend (spec #470, ADR 0020).
//!
//! These exercise the `chat_db` module directly against a real Postgres
//! so the schema, the unique constraint, the cascading delete, and the
//! full-text search all stay honest. The handler-side surface is thin
//! enough that we keep the test focus on the DB layer; the handler's
//! authorization shape (require_workspace_access + scope normalization)
//! is covered by the lib-level unit tests in `handlers::chat::tests`.
//!
//! Skipped when `DATABASE_URL` is unset.

use onsager_portal::chat_db;
use sqlx::{PgPool, Row};
use uuid::Uuid;

async fn try_pool() -> Option<PgPool> {
    let url = std::env::var("DATABASE_URL").ok()?;
    Some(PgPool::connect(&url).await.expect("portal connect"))
}

async fn cleanup_user(pool: &PgPool, user_id: &str) {
    // ON DELETE CASCADE on chat_messages.thread_id handles message rows.
    let _ = sqlx::query("DELETE FROM chat_threads WHERE user_id = $1")
        .bind(user_id)
        .execute(pool)
        .await;
}

/// Round-trip: create thread (implicit on first read), append a message,
/// list it back.
#[tokio::test]
async fn round_trip_thread_and_message() {
    let Some(pool) = try_pool().await else {
        eprintln!("skipping: DATABASE_URL not set");
        return;
    };
    let user_id = format!("u-{}", Uuid::new_v4());
    let workspace_id = format!("ws-{}", Uuid::new_v4());

    let thread = chat_db::get_or_create_thread(&pool, &user_id, &workspace_id, "workspace", None)
        .await
        .expect("create workspace thread");
    assert_eq!(thread.user_id, user_id);
    assert_eq!(thread.workspace_id, workspace_id);
    assert_eq!(thread.scope_type, "workspace");
    assert_eq!(thread.scope_id, None);

    let msg = chat_db::append_message(&pool, &thread.id, "user", "hello world", None)
        .await
        .expect("append message");
    assert_eq!(msg.role, "user");
    assert_eq!(msg.content, "hello world");

    let msgs = chat_db::list_messages(&pool, &thread.id)
        .await
        .expect("list messages");
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0].id, msg.id);

    cleanup_user(&pool, &user_id).await;
}

/// Two get_or_create calls for the same `(user, workspace, scope, scope_id)`
/// return the *same* thread — the upsert is keyed on the unique
/// constraint. Verifies "one rolling thread per scope" (ADR 0020).
#[tokio::test]
async fn scope_uniqueness_enforced() {
    let Some(pool) = try_pool().await else {
        eprintln!("skipping: DATABASE_URL not set");
        return;
    };
    let user_id = format!("u-{}", Uuid::new_v4());
    let workspace_id = format!("ws-{}", Uuid::new_v4());

    let t1 =
        chat_db::get_or_create_thread(&pool, &user_id, &workspace_id, "workflow", Some("wf-A"))
            .await
            .unwrap();
    let t2 =
        chat_db::get_or_create_thread(&pool, &user_id, &workspace_id, "workflow", Some("wf-A"))
            .await
            .unwrap();
    assert_eq!(t1.id, t2.id, "same scope key must return the same thread");

    // Different scope_id produces a distinct thread.
    let t3 =
        chat_db::get_or_create_thread(&pool, &user_id, &workspace_id, "workflow", Some("wf-B"))
            .await
            .unwrap();
    assert_ne!(t1.id, t3.id);

    // Workspace scope (NULL scope_id) is its own row too.
    let t4 = chat_db::get_or_create_thread(&pool, &user_id, &workspace_id, "workspace", None)
        .await
        .unwrap();
    assert_ne!(t1.id, t4.id);
    let t4_again = chat_db::get_or_create_thread(&pool, &user_id, &workspace_id, "workspace", None)
        .await
        .unwrap();
    assert_eq!(
        t4.id, t4_again.id,
        "workspace-scope NULL collapse must dedupe via scope_id_key"
    );

    cleanup_user(&pool, &user_id).await;
}

/// Cross-scope search returns matches with their source scope cited.
#[tokio::test]
async fn cross_scope_search_cites_source_scope() {
    let Some(pool) = try_pool().await else {
        eprintln!("skipping: DATABASE_URL not set");
        return;
    };
    let user_id = format!("u-{}", Uuid::new_v4());
    let workspace_id = format!("ws-{}", Uuid::new_v4());

    let t_workflow =
        chat_db::get_or_create_thread(&pool, &user_id, &workspace_id, "workflow", Some("wf-A"))
            .await
            .unwrap();
    chat_db::append_message(
        &pool,
        &t_workflow.id,
        "user",
        "release notes for the storage migration",
        None,
    )
    .await
    .unwrap();

    let t_run = chat_db::get_or_create_thread(&pool, &user_id, &workspace_id, "run", Some("run-1"))
        .await
        .unwrap();
    chat_db::append_message(&pool, &t_run.id, "user", "unrelated chatter", None)
        .await
        .unwrap();

    let hits = chat_db::search_messages(&pool, &user_id, &workspace_id, "release notes", 50)
        .await
        .unwrap();
    assert_eq!(hits.len(), 1, "only the workflow-scope hit should match");
    let hit = &hits[0];
    assert_eq!(hit.scope_type, "workflow");
    assert_eq!(hit.scope_id.as_deref(), Some("wf-A"));
    assert!(hit.content.contains("release notes"));

    cleanup_user(&pool, &user_id).await;
}

/// Workspace isolation: searches scoped to W1 must not return threads
/// the same user owns in W2, and threads owned by another user in W1
/// must not appear either. This is the cross-workspace + cross-user
/// isolation contract.
///
/// Acts as the "mutation check" guard from the spec's Test section: if
/// the workspace filter on the search SQL were removed, the W1-scoped
/// search would surface the W2 message and this assertion would fail.
#[tokio::test]
async fn workspace_and_user_isolation_enforced() {
    let Some(pool) = try_pool().await else {
        eprintln!("skipping: DATABASE_URL not set");
        return;
    };
    let user_a = format!("u-a-{}", Uuid::new_v4());
    let user_b = format!("u-b-{}", Uuid::new_v4());
    let ws1 = format!("ws1-{}", Uuid::new_v4());
    let ws2 = format!("ws2-{}", Uuid::new_v4());

    // User A — one message in W1, another in W2 (both workspace scope).
    let a_w1 = chat_db::get_or_create_thread(&pool, &user_a, &ws1, "workspace", None)
        .await
        .unwrap();
    chat_db::append_message(&pool, &a_w1.id, "user", "needle in W1", None)
        .await
        .unwrap();
    let a_w2 = chat_db::get_or_create_thread(&pool, &user_a, &ws2, "workspace", None)
        .await
        .unwrap();
    chat_db::append_message(&pool, &a_w2.id, "user", "needle in W2", None)
        .await
        .unwrap();

    // User B — a third message in W1; same workspace as user A's W1
    // thread, but a different user. Must not appear in user A's search.
    let b_w1 = chat_db::get_or_create_thread(&pool, &user_b, &ws1, "workspace", None)
        .await
        .unwrap();
    chat_db::append_message(&pool, &b_w1.id, "user", "needle from user B", None)
        .await
        .unwrap();

    // User A searches W1 — sees only their own W1 message.
    let hits_a_w1 = chat_db::search_messages(&pool, &user_a, &ws1, "needle", 50)
        .await
        .unwrap();
    assert_eq!(hits_a_w1.len(), 1, "A in W1 should see exactly one needle");
    assert!(
        hits_a_w1[0].content.contains("W1"),
        "must be the W1 message, not W2 or user B's"
    );

    // User A searches W2 — sees only their W2 message.
    let hits_a_w2 = chat_db::search_messages(&pool, &user_a, &ws2, "needle", 50)
        .await
        .unwrap();
    assert_eq!(hits_a_w2.len(), 1);
    assert!(hits_a_w2[0].content.contains("W2"));

    // User B searches W1 — sees only their own message.
    let hits_b_w1 = chat_db::search_messages(&pool, &user_b, &ws1, "needle", 50)
        .await
        .unwrap();
    assert_eq!(hits_b_w1.len(), 1);
    assert!(hits_b_w1[0].content.contains("user B"));

    cleanup_user(&pool, &user_a).await;
    cleanup_user(&pool, &user_b).await;
}

/// Cascade delete: removing a thread row drops its messages.
#[tokio::test]
async fn delete_thread_cascades_to_messages() {
    let Some(pool) = try_pool().await else {
        eprintln!("skipping: DATABASE_URL not set");
        return;
    };
    let user_id = format!("u-{}", Uuid::new_v4());
    let workspace_id = format!("ws-{}", Uuid::new_v4());

    let thread = chat_db::get_or_create_thread(&pool, &user_id, &workspace_id, "workspace", None)
        .await
        .unwrap();
    chat_db::append_message(&pool, &thread.id, "user", "to be deleted", None)
        .await
        .unwrap();

    sqlx::query("DELETE FROM chat_threads WHERE id = $1")
        .bind(&thread.id)
        .execute(&pool)
        .await
        .unwrap();

    let count: i64 = sqlx::query("SELECT COUNT(*) FROM chat_messages WHERE thread_id = $1")
        .bind(&thread.id)
        .fetch_one(&pool)
        .await
        .unwrap()
        .get(0);
    assert_eq!(count, 0, "messages must cascade-delete with the thread");
}
