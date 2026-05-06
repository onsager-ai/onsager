//! Integration tests for stiglab's PAT-aware `AuthUser` extractor.
//!
//! Spec #222 Slice 2b moved `/api/pats*` CRUD to portal. Slices 3–4 moved
//! workspaces, projects, workflows. Follow-up 3 + Slice 6 moved sessions,
//! nodes, tasks and completed the cutover so stiglab owns no auth-gated HTTP
//! routes at all (only `/api/health` and `/agent/ws`).
//!
//! The end-to-end HTTP-level PAT auth tests that pivoted onto
//! `/api/sessions?workspace={id}` were retired here after that route moved
//! to portal in Follow-up 3 — there is no longer a suitable stiglab-owned
//! route to exercise the `AuthUser` extractor over HTTP without a running
//! portal + Postgres harness. The tests below cover the DB primitives that
//! remain stiglab-local: `verify_pat`, hash invariants, and token format.

use chrono::Utc;
use sqlx::pool::PoolOptions;
use sqlx::AnyPool;
use stiglab::core::{User, Workspace, WorkspaceMember};
use stiglab::server::auth::{generate_pat_token, hash_pat_token, PAT_PREFIX_LEN};
use stiglab::server::db;
use uuid::Uuid;

async fn test_pool() -> AnyPool {
    sqlx::any::install_default_drivers();
    let pool = PoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("sqlite connect");
    db::run_migrations(&pool).await.expect("migrations");
    pool
}

async fn seed_user(pool: &AnyPool) -> User {
    let user = User {
        id: Uuid::new_v4().to_string(),
        github_id: 7,
        github_login: "patuser".into(),
        github_name: Some("PAT User".into()),
        github_avatar_url: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    db::upsert_user(pool, &user).await.unwrap();
    user
}

async fn seed_workspace_with_member(pool: &AnyPool, user_id: &str, slug: &str) -> Workspace {
    let now = Utc::now();
    let workspace = Workspace {
        id: Uuid::new_v4().to_string(),
        slug: slug.into(),
        name: slug.into(),
        created_by: user_id.into(),
        created_at: now,
    };
    let member = WorkspaceMember {
        workspace_id: workspace.id.clone(),
        user_id: user_id.into(),
        joined_at: now,
    };
    db::insert_workspace_with_creator(pool, &workspace, &member)
        .await
        .unwrap();
    workspace
}

/// Mint a PAT directly via the DB layer, bypassing `POST /api/pats` (which
/// now lives on portal). Used to set up auth state before exercising other
/// endpoints.
///
/// PATs are workspace-scoped post-#163; when the caller doesn't already have
/// a workspace in scope, pass `None` and `mint_pat` seeds a throwaway one
/// for the user just so the row satisfies the NOT NULL constraint.
async fn mint_pat(
    pool: &AnyPool,
    user_id: &str,
    workspace_id: Option<&str>,
    name: &str,
    expires_at: Option<chrono::DateTime<Utc>>,
) -> (String, String) {
    let synthetic_ws;
    let workspace_id: &str = match workspace_id {
        Some(w) => w,
        None => {
            let ws = seed_workspace_with_member(
                pool,
                user_id,
                &format!("pat-{}", &Uuid::new_v4().to_string()[..8]),
            )
            .await;
            synthetic_ws = ws.id;
            synthetic_ws.as_str()
        }
    };
    let generated = generate_pat_token();
    let id = Uuid::new_v4().to_string();
    db::insert_user_pat(
        pool,
        &id,
        user_id,
        workspace_id,
        name,
        &generated.prefix,
        &generated.hash,
        expires_at,
    )
    .await
    .unwrap();
    (id, generated.token)
}

// ── Token format + hash invariants ──

#[test]
fn token_has_correct_prefix_and_length() {
    let pat = generate_pat_token();
    assert!(pat.token.starts_with("ons_pat_"));
    assert_eq!(pat.prefix.len(), PAT_PREFIX_LEN);
    assert_eq!(&pat.prefix, &pat.token[..PAT_PREFIX_LEN]);
    assert_eq!(pat.hash, hash_pat_token(&pat.token));
}

// ── DB-level verify_pat ──

#[tokio::test]
async fn verify_pat_rejects_unknown_prefix() {
    let pool = test_pool().await;
    let outcome = stiglab::server::auth::verify_pat(&pool, "ons_pat_doesnotexistxyz123abc")
        .await
        .unwrap();
    assert!(matches!(
        outcome,
        stiglab::server::auth::PatVerifyOutcome::Unknown
    ));
}

#[tokio::test]
async fn verify_pat_rejects_wrong_namespace() {
    let pool = test_pool().await;
    let outcome = stiglab::server::auth::verify_pat(&pool, "ghp_aaaaaaaaaaaaaaaa")
        .await
        .unwrap();
    assert!(matches!(
        outcome,
        stiglab::server::auth::PatVerifyOutcome::Unknown
    ));
}

#[tokio::test]
async fn verify_pat_accepts_valid_token() {
    let pool = test_pool().await;
    let user = seed_user(&pool).await;
    let (pat_id, token) = mint_pat(&pool, &user.id, None, "ci", None).await;
    let outcome = stiglab::server::auth::verify_pat(&pool, &token)
        .await
        .unwrap();
    match outcome {
        stiglab::server::auth::PatVerifyOutcome::Ok(pat) => {
            assert_eq!(pat.id, pat_id);
            assert_eq!(pat.user_id, user.id);
        }
        other => panic!("expected Ok, got {other:?}"),
    }
}

#[tokio::test]
async fn verify_pat_reports_revoked_separately_from_unknown() {
    let pool = test_pool().await;
    let user = seed_user(&pool).await;
    let (pat_id, token) = mint_pat(&pool, &user.id, None, "ci", None).await;
    db::revoke_user_pat(&pool, &user.id, &pat_id).await.unwrap();
    let outcome = stiglab::server::auth::verify_pat(&pool, &token)
        .await
        .unwrap();
    assert!(matches!(
        outcome,
        stiglab::server::auth::PatVerifyOutcome::Revoked
    ));
}

#[tokio::test]
async fn verify_pat_reports_expired_separately_from_unknown() {
    let pool = test_pool().await;
    let user = seed_user(&pool).await;
    let past = Utc::now() - chrono::Duration::seconds(60);
    let (_, token) = mint_pat(&pool, &user.id, None, "ci", Some(past)).await;
    let outcome = stiglab::server::auth::verify_pat(&pool, &token)
        .await
        .unwrap();
    assert!(matches!(
        outcome,
        stiglab::server::auth::PatVerifyOutcome::Expired
    ));
}
