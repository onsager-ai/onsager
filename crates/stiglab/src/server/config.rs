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

#[cfg(test)]
mod tests {
    use super::*;

    fn make_config(client_id: Option<&str>, client_secret: Option<&str>) -> ServerConfig {
        ServerConfig {
            host: "0.0.0.0".to_string(),
            port: 3000,
            database_url: "sqlite://test.db".to_string(),
            static_dir: None,
            cors_origin: None,
            github_client_id: client_id.map(|s| s.to_string()),
            github_client_secret: client_secret.map(|s| s.to_string()),
            credential_key: None,
            public_url: None,
        }
    }

    #[test]
    fn auth_enabled_when_both_set() {
        let config = make_config(Some("id"), Some("secret"));
        assert!(config.auth_enabled());
    }

    #[test]
    fn auth_disabled_when_id_missing() {
        let config = make_config(None, Some("secret"));
        assert!(!config.auth_enabled());
    }

    #[test]
    fn auth_disabled_when_secret_missing() {
        let config = make_config(Some("id"), None);
        assert!(!config.auth_enabled());
    }

    #[test]
    fn auth_disabled_when_both_missing() {
        let config = make_config(None, None);
        assert!(!config.auth_enabled());
    }
}
