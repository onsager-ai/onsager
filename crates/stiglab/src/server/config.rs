use std::env;

/// Runtime configuration for the stiglab server.
///
/// Auth-related fields (`github_client_id`, `github_client_secret`,
/// `sso_*`) used to live here. They moved to portal in spec #222 Slice 5
/// — portal owns `/api/auth/*` and is the only process that talks to
/// GitHub OAuth. Stiglab still validates the cookie out-of-band against
/// the shared `auth_sessions` table (single DB, single writer).
#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub database_url: String,
    pub static_dir: Option<String>,
    pub cors_origin: Option<String>,
    pub credential_key: Option<String>,
    pub public_url: Option<String>,
    /// Shared secret between forge (caller) and stiglab (callee) on the
    /// internal `/api/shaping` HTTP path (issue #156). When set, only
    /// requests carrying this token in the `X-Onsager-Internal-Dispatch`
    /// header may set `created_by` and have stiglab decrypt that user's
    /// credentials. Without it, attackers reaching the public endpoint
    /// could exfiltrate any user's tokens by guessing their user_id.
    /// Goes away with the seam (Lever C, spec #131 / #148) once
    /// shaping moves to spine events.
    pub internal_dispatch_token: Option<String>,
}

impl ServerConfig {
    pub fn from_env() -> Self {
        let host = env::var("STIGLAB_HOST").unwrap_or_else(|_| "0.0.0.0".to_string());
        let port = env::var("PORT")
            .ok()
            .and_then(|p| p.parse().ok())
            .or_else(|| env::var("STIGLAB_PORT").ok().and_then(|p| p.parse().ok()))
            .unwrap_or(3000);
        let database_url =
            env::var("DATABASE_URL").unwrap_or_else(|_| "sqlite://./data/stiglab.db".to_string());
        let static_dir = env::var("STIGLAB_STATIC_DIR").ok();
        let cors_origin = env::var("STIGLAB_CORS_ORIGIN").ok();
        let credential_key = env::var("STIGLAB_CREDENTIAL_KEY").ok();
        let public_url = env::var("STIGLAB_PUBLIC_URL").ok();
        let internal_dispatch_token = env::var("STIGLAB_INTERNAL_DISPATCH_TOKEN")
            .ok()
            .filter(|s| !s.is_empty());

        ServerConfig {
            host,
            port,
            database_url,
            static_dir,
            cors_origin,
            credential_key,
            public_url,
            internal_dispatch_token,
        }
    }
}
