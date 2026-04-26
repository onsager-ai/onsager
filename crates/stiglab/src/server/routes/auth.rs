use axum::extract::{Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use axum::Json;
use chrono::Utc;
use uuid::Uuid;

use crate::core::User;

use crate::server::auth::{
    self, exchange_code, generate_session_token, generate_state, get_github_user,
    github_authorize_url, AuthUser, RequestPrincipal,
};
use crate::server::db;
use crate::server::sso::{
    self, generate_exchange_code, return_to_allowed, secrets_equal, sign_state, verify_state,
    SsoMode, StateClaims, EXCHANGE_CODE_LIFETIME_SECS, STATE_LIFETIME_SECS,
};
use crate::server::state::AppState;

// ── GitHub OAuth entry + callback (owner) / redirect-to-owner (relying) ──

#[derive(serde::Deserialize)]
pub struct LoginParams {
    /// Optional URL the owner should 302 to after a successful sign-in,
    /// in place of minting a local session. Only honored when
    /// `return_to`'s host is in the owner's allowlist.
    return_to: Option<String>,
}

#[derive(serde::Deserialize)]
pub struct CallbackParams {
    code: String,
    state: String,
}

/// GET /api/auth/github — Start sign-in.
///
/// * Owner mode: redirect to GitHub with a signed state envelope.
/// * Relying mode: redirect to the owner's `/api/auth/github`, passing our
///   own `/api/auth/sso/finish` as `return_to`.
/// * Disabled: 404.
pub async fn github_login(
    State(state): State<AppState>,
    Query(params): Query<LoginParams>,
) -> impl IntoResponse {
    let config = &state.config;

    match config.sso_mode() {
        SsoMode::Disabled => (StatusCode::NOT_FOUND, "auth not configured").into_response(),
        SsoMode::Relying => {
            // Previews never talk to GitHub directly — bounce through prod.
            let auth_domain = config
                .sso_auth_domain
                .as_deref()
                .expect("relying mode implies sso_auth_domain set");
            let Some(public_url) = config.public_url.as_deref() else {
                tracing::error!(
                    "cannot initiate relying-mode SSO: STIGLAB_PUBLIC_URL is unset, \
                     so we have nowhere to tell the owner to send the browser back to"
                );
                return (StatusCode::INTERNAL_SERVER_ERROR, "auth misconfigured").into_response();
            };
            let return_to = format!("{public_url}/api/auth/sso/finish");
            let target = match reqwest::Url::parse_with_params(
                &format!("{auth_domain}/api/auth/github"),
                &[("return_to", return_to.as_str())],
            ) {
                Ok(u) => u.to_string(),
                Err(e) => {
                    tracing::error!("failed to build relying-mode redirect URL: {e}");
                    return (StatusCode::INTERNAL_SERVER_ERROR, "auth misconfigured")
                        .into_response();
                }
            };
            Redirect::temporary(&target).into_response()
        }
        SsoMode::Owner { delegate_enabled } => {
            let Some(client_id) = config.github_client_id.as_deref() else {
                return (StatusCode::NOT_FOUND, "auth not configured").into_response();
            };

            // If a `return_to` was provided, it must pass the allowlist
            // before we even involve GitHub. We reject here rather than at
            // the callback so misconfigured relying parties fail fast.
            let return_to = match params.return_to {
                None => None,
                Some(ref rt) => {
                    if !delegate_enabled {
                        return (
                            StatusCode::FORBIDDEN,
                            "cross-environment SSO delegation is not enabled on this owner",
                        )
                            .into_response();
                    }
                    if !return_to_allowed(&config.sso_return_host_allowlist, rt) {
                        tracing::warn!(return_to = %rt, "rejected SSO delegation: return_to not on allowlist");
                        return (StatusCode::FORBIDDEN, "return_to not allowed").into_response();
                    }
                    Some(rt.clone())
                }
            };

            let csrf_nonce = generate_state();
            let now = Utc::now().timestamp();
            let claims = StateClaims {
                c: csrf_nonce.clone(),
                r: return_to,
                e: now + STATE_LIFETIME_SECS,
            };

            // When delegation is enabled we HMAC-sign the envelope so the
            // callback can trust the return_to. When only simple OAuth is
            // needed, fall back to a bare nonce to preserve existing
            // behavior on owners that haven't opted into delegation.
            let state_param = if delegate_enabled {
                let secret = config
                    .sso_state_secret
                    .as_deref()
                    .expect("delegate_enabled implies sso_state_secret set");
                sign_state(secret, &claims)
            } else {
                csrf_nonce.clone()
            };

            let callback_url = build_callback_url(config);
            let url = github_authorize_url(client_id, &callback_url, &state_param);

            let sec = secure_attr(config);
            let cookie = format!(
                "stiglab_oauth_state={csrf_nonce}; Path=/; HttpOnly; SameSite=Lax; Max-Age=600{sec}"
            );

            ([(header::SET_COOKIE, cookie)], Redirect::temporary(&url)).into_response()
        }
    }
}

/// GET /api/auth/github/callback — Handle OAuth callback.
///
/// In owner mode, exchanges the code for a GitHub token and either mints
/// a local session (standalone login) or mints an exchange code and 302s
/// to the return_to URL (delegated login for a preview).
pub async fn github_callback(
    State(state): State<AppState>,
    Query(params): Query<CallbackParams>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let config = &state.config;

    let SsoMode::Owner { delegate_enabled } = config.sso_mode() else {
        return (StatusCode::NOT_FOUND, "auth not configured").into_response();
    };

    let (Some(client_id), Some(client_secret)) = (
        config.github_client_id.as_deref(),
        config.github_client_secret.as_deref(),
    ) else {
        return (StatusCode::NOT_FOUND, "auth not configured").into_response();
    };

    // Parse CSRF cookie once — used by both branches below.
    let cookie_header = headers
        .get(header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let cookie_csrf = auth::parse_cookie(cookie_header, "stiglab_oauth_state");

    // Delegated login carries an HMAC-signed state envelope; standalone
    // login carries a bare nonce that must equal the CSRF cookie.
    let claims: Option<StateClaims> = if delegate_enabled {
        let secret = config
            .sso_state_secret
            .as_deref()
            .expect("delegate_enabled implies sso_state_secret set");
        verify_state(secret, &params.state, Utc::now().timestamp())
    } else {
        None
    };

    let delegated_return_to: Option<String> = match claims {
        Some(ref c) => {
            if cookie_csrf != Some(c.c.as_str()) {
                return (StatusCode::BAD_REQUEST, "invalid OAuth state").into_response();
            }
            // If the envelope carries a return_to, double-check the
            // allowlist at redemption time — the allowlist may have been
            // tightened between start and callback.
            if let Some(ref rt) = c.r {
                if !return_to_allowed(&config.sso_return_host_allowlist, rt) {
                    tracing::warn!(return_to = %rt, "callback rejected delegated SSO: return_to no longer allowed");
                    return (StatusCode::FORBIDDEN, "return_to not allowed").into_response();
                }
            }
            c.r.clone()
        }
        None => {
            // Bare-nonce path — matches the pre-delegation behavior.
            if cookie_csrf != Some(params.state.as_str()) {
                return (StatusCode::BAD_REQUEST, "invalid OAuth state").into_response();
            }
            None
        }
    };

    // Exchange code for token
    let token = match exchange_code(client_id, client_secret, &params.code).await {
        Ok(t) => t,
        Err(e) => {
            tracing::error!("GitHub token exchange failed: {e}");
            return (StatusCode::BAD_GATEWAY, "GitHub authentication failed").into_response();
        }
    };

    // Fetch GitHub user profile
    let gh_user = match get_github_user(&token.access_token).await {
        Ok(u) => u,
        Err(e) => {
            tracing::error!("GitHub user API failed: {e}");
            return (StatusCode::BAD_GATEWAY, "Failed to fetch GitHub profile").into_response();
        }
    };

    // Upsert user in DB
    let existing = db::get_user_by_github_id(&state.db, gh_user.id)
        .await
        .ok()
        .flatten();
    let user_id = existing
        .as_ref()
        .map(|u| u.id.clone())
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    let user = User {
        id: user_id.clone(),
        github_id: gh_user.id,
        github_login: gh_user.login.clone(),
        github_name: gh_user.name,
        github_avatar_url: gh_user.avatar_url,
        created_at: existing.map(|u| u.created_at).unwrap_or_else(Utc::now),
        updated_at: Utc::now(),
    };

    if let Err(e) = db::upsert_user(&state.db, &user).await {
        tracing::error!("failed to upsert user: {e}");
        return (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response();
    }

    let sec = secure_attr(config);
    let clear_state_cookie =
        format!("stiglab_oauth_state=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0{sec}");

    if let Some(return_to) = delegated_return_to {
        // Delegated login: mint a single-use exchange code and 302 home.
        // The relying party redeems it server-to-server in `sso_finish`.
        let host = match sso::host_of(&return_to) {
            Some(h) => h,
            None => {
                return (StatusCode::BAD_REQUEST, "invalid return_to").into_response();
            }
        };
        let code = generate_exchange_code();
        let expires_at = Utc::now() + chrono::Duration::seconds(EXCHANGE_CODE_LIFETIME_SECS);
        if let Err(e) =
            db::insert_sso_exchange_code(&state.db, &code, &user_id, &host, expires_at).await
        {
            tracing::error!("failed to insert sso exchange code: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response();
        }

        tracing::info!(
            user = %user.github_login,
            return_to_host = %host,
            "sso delegation: issued exchange code"
        );

        let sep = if return_to.contains('?') { '&' } else { '?' };
        let location = format!("{return_to}{sep}code={code}");
        return Response::builder()
            .status(StatusCode::FOUND)
            .header(header::LOCATION, location)
            .header(header::SET_COOKIE, clear_state_cookie)
            .body(axum::body::Body::empty())
            .unwrap()
            .into_response();
    }

    // Standalone login: mint a local session.
    let session_token = generate_session_token();
    let expires_at = Utc::now() + chrono::Duration::days(30);

    if let Err(e) = db::create_auth_session(&state.db, &session_token, &user_id, expires_at).await {
        tracing::error!("failed to create auth session: {e}");
        return (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response();
    }

    tracing::info!("user logged in: {} ({})", user.github_login, user_id);

    let session_cookie = format!(
        "stiglab_session={session_token}; Path=/; HttpOnly; SameSite=Lax; Max-Age=2592000{sec}"
    );

    Response::builder()
        .status(StatusCode::FOUND)
        .header(header::LOCATION, "/")
        .header(header::SET_COOKIE, session_cookie)
        .header(header::SET_COOKIE, clear_state_cookie)
        .body(axum::body::Body::empty())
        .unwrap()
        .into_response()
}

// ── Owner-side: /api/auth/sso/redeem (back-channel) ──

#[derive(serde::Deserialize)]
pub struct SsoRedeemRequest {
    pub code: String,
    /// The host the caller is claiming the code was issued for. Must match
    /// the host baked into the code at issuance.
    pub host: String,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct SsoRedeemResponse {
    pub user: SsoUserPayload,
}

#[derive(serde::Serialize, serde::Deserialize, Debug)]
pub struct SsoUserPayload {
    pub github_id: i64,
    pub github_login: String,
    pub github_name: Option<String>,
    pub github_avatar_url: Option<String>,
}

/// POST /api/auth/sso/redeem — Exchange an opaque code for user identity.
///
/// Called server-to-server by a relying party. Requires
/// `Authorization: Bearer <SSO_EXCHANGE_SECRET>`. Returns 404 when this
/// process is not an owner with delegation enabled — the route simply
/// doesn't exist on a standalone deploy.
pub async fn sso_redeem(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<SsoRedeemRequest>,
) -> impl IntoResponse {
    let config = &state.config;

    let SsoMode::Owner {
        delegate_enabled: true,
    } = config.sso_mode()
    else {
        return (StatusCode::NOT_FOUND, "not found").into_response();
    };

    let expected_bearer = config
        .sso_exchange_secret
        .as_deref()
        .expect("delegate_enabled implies sso_exchange_secret set");
    if !bearer_matches(&headers, expected_bearer) {
        return (StatusCode::UNAUTHORIZED, "invalid bearer").into_response();
    }

    match db::redeem_sso_exchange_code(&state.db, &body.code, &body.host).await {
        Ok(Some(user)) => {
            tracing::info!(
                user = %user.github_login,
                host = %body.host,
                "sso delegation: redeemed exchange code"
            );
            Json(SsoRedeemResponse {
                user: SsoUserPayload {
                    github_id: user.github_id,
                    github_login: user.github_login,
                    github_name: user.github_name,
                    github_avatar_url: user.github_avatar_url,
                },
            })
            .into_response()
        }
        Ok(None) => (StatusCode::BAD_REQUEST, "invalid or expired code").into_response(),
        Err(e) => {
            tracing::error!("sso_redeem db error: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response()
        }
    }
}

fn bearer_matches(headers: &HeaderMap, expected: &str) -> bool {
    let Some(header_val) = headers.get(header::AUTHORIZATION) else {
        return false;
    };
    let Ok(header_str) = header_val.to_str() else {
        return false;
    };
    let Some(token) = header_str.strip_prefix("Bearer ") else {
        return false;
    };
    secrets_equal(token.trim(), expected)
}

// ── Relying-side: /api/auth/sso/finish ──

#[derive(serde::Deserialize)]
pub struct SsoFinishParams {
    code: String,
}

/// GET /api/auth/sso/finish — Relying-side landing after the owner
/// completes the OAuth dance.
///
/// Server-to-server call to the owner's `/sso/redeem`, then mint a local
/// session and set the `stiglab_session` cookie on our own origin.
pub async fn sso_finish(
    State(state): State<AppState>,
    Query(params): Query<SsoFinishParams>,
) -> impl IntoResponse {
    let config = &state.config;

    if config.sso_mode() != SsoMode::Relying {
        return (StatusCode::NOT_FOUND, "not found").into_response();
    }

    let auth_domain = config
        .sso_auth_domain
        .as_deref()
        .expect("relying implies sso_auth_domain set");
    let exchange_secret = config
        .sso_exchange_secret
        .as_deref()
        .expect("relying implies sso_exchange_secret set");
    let Some(public_url) = config.public_url.as_deref() else {
        tracing::error!("sso_finish: STIGLAB_PUBLIC_URL is unset");
        return (StatusCode::INTERNAL_SERVER_ERROR, "auth misconfigured").into_response();
    };

    let Some(our_host) = sso::host_of(public_url) else {
        tracing::error!(public_url, "sso_finish: STIGLAB_PUBLIC_URL has no host");
        return (StatusCode::INTERNAL_SERVER_ERROR, "auth misconfigured").into_response();
    };

    let redeem_url = format!("{auth_domain}/api/auth/sso/redeem");
    let body = serde_json::json!({ "code": params.code, "host": our_host });

    let client = reqwest::Client::new();
    let resp = match client
        .post(&redeem_url)
        .bearer_auth(exchange_secret)
        .json(&body)
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("sso_finish: redeem request failed: {e}");
            return (StatusCode::BAD_GATEWAY, "owner unreachable").into_response();
        }
    };

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        tracing::error!(status = %status, body = %text, "sso_finish: redeem rejected");
        // Owner 4xx ("expired/redeemed/unknown code", "bad bearer") is a
        // client-facing problem with the preview's request — propagate as
        // 400. Reserve 502 for real upstream failures (5xx, timeouts,
        // connectivity) so the preview's browser-visible status matches
        // the underlying condition.
        let mapped = if status.is_client_error() {
            StatusCode::BAD_REQUEST
        } else {
            StatusCode::BAD_GATEWAY
        };
        return (mapped, "redeem rejected").into_response();
    }

    let payload: SsoRedeemResponse = match resp.json().await {
        Ok(p) => p,
        Err(e) => {
            tracing::error!("sso_finish: failed to parse redeem response: {e}");
            return (StatusCode::BAD_GATEWAY, "bad redeem response").into_response();
        }
    };

    // Upsert the user locally. The preview maintains its own users table;
    // the github_id is the stable key across environments.
    let existing = db::get_user_by_github_id(&state.db, payload.user.github_id)
        .await
        .ok()
        .flatten();
    let user_id = existing
        .as_ref()
        .map(|u| u.id.clone())
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    let user = User {
        id: user_id.clone(),
        github_id: payload.user.github_id,
        github_login: payload.user.github_login.clone(),
        github_name: payload.user.github_name,
        github_avatar_url: payload.user.github_avatar_url,
        created_at: existing.map(|u| u.created_at).unwrap_or_else(Utc::now),
        updated_at: Utc::now(),
    };

    if let Err(e) = db::upsert_user(&state.db, &user).await {
        tracing::error!("sso_finish: failed to upsert user: {e}");
        return (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response();
    }

    let session_token = generate_session_token();
    let expires_at = Utc::now() + chrono::Duration::days(30);
    if let Err(e) = db::create_auth_session(&state.db, &session_token, &user_id, expires_at).await {
        tracing::error!("sso_finish: failed to create auth session: {e}");
        return (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response();
    }

    tracing::info!(
        user = %user.github_login,
        host = %our_host,
        "sso delegation: minted preview session"
    );

    let sec = secure_attr(config);
    let session_cookie = format!(
        "stiglab_session={session_token}; Path=/; HttpOnly; SameSite=Lax; Max-Age=2592000{sec}"
    );

    Response::builder()
        .status(StatusCode::FOUND)
        .header(header::LOCATION, "/")
        .header(header::SET_COOKIE, session_cookie)
        .body(axum::body::Body::empty())
        .unwrap()
        .into_response()
}

// ── /api/auth/me + logout (unchanged behavior) ──

/// GET /api/auth/me — Return current authenticated user
pub async fn me(State(state): State<AppState>, auth_user: AuthUser) -> impl IntoResponse {
    let via = match auth_user.principal {
        RequestPrincipal::Pat { .. } => "pat",
        RequestPrincipal::Session => "session",
    };
    Json(serde_json::json!({
        "user": {
            "id": auth_user.user_id,
            "github_login": auth_user.github_login,
            "github_name": auth_user.github_name,
            "github_avatar_url": auth_user.github_avatar_url,
        },
        "auth_enabled": state.config.auth_enabled(),
        "via": via,
    }))
}

/// POST /api/auth/logout — Delete auth session and clear cookie
pub async fn logout(State(state): State<AppState>, headers: HeaderMap) -> impl IntoResponse {
    let cookie_header = headers
        .get(header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if let Some(session_id) = auth::parse_cookie(cookie_header, "stiglab_session") {
        let _ = db::delete_auth_session(&state.db, session_id).await;
    }

    let sec = secure_attr(&state.config);
    let clear_cookie = format!("stiglab_session=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0{sec}");

    (
        [(header::SET_COOKIE, clear_cookie)],
        Json(serde_json::json!({ "ok": true })),
    )
}

fn build_callback_url(config: &crate::server::config::ServerConfig) -> String {
    if let Some(ref public_url) = config.public_url {
        format!("{public_url}/api/auth/github/callback")
    } else {
        format!("http://localhost:{}/api/auth/github/callback", config.port)
    }
}

/// Returns "; Secure" if the public URL uses HTTPS, empty string otherwise.
fn secure_attr(config: &crate::server::config::ServerConfig) -> &'static str {
    if config
        .public_url
        .as_deref()
        .is_some_and(|u| u.starts_with("https://"))
    {
        "; Secure"
    } else {
        ""
    }
}
