use std::env;

#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub database_url: String,
    pub static_dir: Option<String>,
    pub cors_origin: Option<String>,
    pub github_client_id: Option<String>,
    pub github_client_secret: Option<String>,
    pub credential_key: Option<String>,
    pub public_url: Option<String>,
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
        let github_client_id = env::var("GITHUB_CLIENT_ID").ok();
        let github_client_secret = env::var("GITHUB_CLIENT_SECRET").ok();
        let credential_key = env::var("STIGLAB_CREDENTIAL_KEY").ok();
        let public_url = env::var("STIGLAB_PUBLIC_URL").ok();

        ServerConfig {
            host,
            port,
            database_url,
            static_dir,
            cors_origin,
            github_client_id,
            github_client_secret,
            credential_key,
            public_url,
        }
    }

    /// Returns true if GitHub OAuth is configured (both client ID and secret are set).
    pub fn auth_enabled(&self) -> bool {
        self.github_client_id.is_some() && self.github_client_secret.is_some()
    }
}
