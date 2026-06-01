//! Integration test for spec #531 — per-session workspace-scoped tokens.
//!
//! Exercises the full mint → encrypt → decrypt → verify → revoke
//! round-trip against `user_pats`:
//!
//! - `mint_for_session` returns the raw token **encrypted** with the
//!   credential key and persists only the hash.
//! - The ciphertext decrypts to a live `ons_pat_` token that `verify_pat`
//!   accepts, pinned to the session's workspace.
//! - `revoke_for_session` flips the row to revoked so `verify_pat` then
//!   reports `Revoked` — the revoke-on-end path the session-lifecycle
//!   listener drives.
//!
//! Skipped when `DATABASE_URL` is unset — `user_pats` is a portal table
//! only the Postgres-backed harness exercises.

use onsager_portal::auth::{
    PatVerifyOutcome, decrypt_credential, generate_credential_key, verify_pat,
};
use onsager_portal::session_token::{
    mint_for_plan, mint_for_session, plan_pat_name, revoke_for_plan, revoke_for_session,
    session_pat_name,
};
use sqlx::PgPool;

async fn try_pool() -> Option<PgPool> {
    let url = std::env::var("DATABASE_URL").ok()?;
    Some(PgPool::connect(&url).await.expect("db connect"))
}

async fn cleanup(pool: &PgPool, session_id: &str) {
    let _ = sqlx::query("DELETE FROM user_pats WHERE name = $1")
        .bind(session_pat_name(session_id))
        .execute(pool)
        .await;
}

async fn cleanup_plan(pool: &PgPool, plan_id: &str) {
    let _ = sqlx::query("DELETE FROM user_pats WHERE name = $1")
        .bind(plan_pat_name(plan_id))
        .execute(pool)
        .await;
}

#[tokio::test]
async fn mint_encrypts_a_verifiable_token_then_revoke_invalidates_it() {
    let Some(pool) = try_pool().await else {
        eprintln!("skipping: DATABASE_URL not set");
        return;
    };

    let key = generate_credential_key();
    let user_id = format!("u_{}", ulid::Ulid::new());
    let workspace_id = format!("ws_{}", ulid::Ulid::new());
    let session_id = ulid::Ulid::new().to_string();
    cleanup(&pool, &session_id).await;

    // Mint: returns the raw token encrypted with the credential key.
    let encrypted = mint_for_session(&pool, &key, &user_id, &workspace_id, &session_id)
        .await
        .expect("mint");

    // The ciphertext is not the token itself, but decrypts to a real PAT.
    assert!(
        !encrypted.starts_with("ons_pat_"),
        "token must travel encrypted"
    );
    let raw = decrypt_credential(&key, &encrypted).expect("decrypt");
    assert!(raw.starts_with("ons_pat_"), "decrypts to a PAT");

    // The decrypted token verifies and is pinned to the session workspace.
    match verify_pat(&pool, &raw).await.expect("verify") {
        PatVerifyOutcome::Ok(pat) => {
            assert_eq!(pat.workspace_id, workspace_id);
            assert_eq!(pat.user_id, user_id);
            assert_eq!(pat.name, session_pat_name(&session_id));
            assert!(pat.expires_at.is_some(), "session token carries an expiry");
        }
        other => panic!("expected Ok, got {other:?}"),
    }

    // Revoke-on-end: the token is invalidated immediately.
    let revoked = revoke_for_session(&pool, &session_id)
        .await
        .expect("revoke");
    assert_eq!(revoked, 1, "exactly one live token revoked");

    match verify_pat(&pool, &raw).await.expect("verify after revoke") {
        PatVerifyOutcome::Revoked => {}
        other => panic!("expected Revoked, got {other:?}"),
    }

    // Idempotent: a second revoke affects no rows.
    let again = revoke_for_session(&pool, &session_id)
        .await
        .expect("revoke again");
    assert_eq!(again, 0, "second revoke is a no-op");

    cleanup(&pool, &session_id).await;
}

/// Spec #536 — the plan-scoped grain mirrors the session grain:
/// `mint_for_plan` → `decrypt_credential` → `verify_pat` (workspace-
/// pinned, has expiry) → `revoke_for_plan` → idempotent second revoke.
/// Same `DATABASE_URL` gate as the session test.
#[tokio::test]
async fn plan_token_mints_verifiable_then_revoke_invalidates_it() {
    let Some(pool) = try_pool().await else {
        eprintln!("skipping: DATABASE_URL not set");
        return;
    };

    let key = generate_credential_key();
    let user_id = format!("u_{}", ulid::Ulid::new());
    let workspace_id = format!("ws_{}", ulid::Ulid::new());
    let plan_id = format!("{workspace_id}:github:{}", ulid::Ulid::new());
    cleanup_plan(&pool, &plan_id).await;

    // The plan-pat name carries the distinct `plan:` prefix.
    assert_eq!(plan_pat_name(&plan_id), format!("plan:{plan_id}"));

    let encrypted = mint_for_plan(&pool, &key, &user_id, &workspace_id, &plan_id)
        .await
        .expect("mint plan token");
    assert!(
        !encrypted.starts_with("ons_pat_"),
        "token must travel encrypted"
    );
    let raw = decrypt_credential(&key, &encrypted).expect("decrypt");
    assert!(raw.starts_with("ons_pat_"), "decrypts to a PAT");

    match verify_pat(&pool, &raw).await.expect("verify") {
        PatVerifyOutcome::Ok(pat) => {
            assert_eq!(pat.workspace_id, workspace_id);
            assert_eq!(pat.user_id, user_id);
            assert_eq!(pat.name, plan_pat_name(&plan_id));
            assert!(pat.expires_at.is_some(), "plan token carries an expiry");
        }
        other => panic!("expected Ok, got {other:?}"),
    }

    let revoked = revoke_for_plan(&pool, &plan_id).await.expect("revoke");
    assert_eq!(revoked, 1, "exactly one live plan token revoked");

    match verify_pat(&pool, &raw).await.expect("verify after revoke") {
        PatVerifyOutcome::Revoked => {}
        other => panic!("expected Revoked, got {other:?}"),
    }

    let again = revoke_for_plan(&pool, &plan_id)
        .await
        .expect("revoke again");
    assert_eq!(again, 0, "second revoke is a no-op");

    cleanup_plan(&pool, &plan_id).await;
}
