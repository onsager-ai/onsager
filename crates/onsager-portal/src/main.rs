//! `onsager-portal` binary entry point — see crate docs for the full
//! ingestion pipeline.

use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(
    name = "onsager-portal",
    about = "GitHub webhook ingress for the Onsager factory"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Run the webhook server.
    Serve {
        /// Bind address (default `0.0.0.0:8080` or `PORTAL_BIND`).
        #[arg(long, env = "PORTAL_BIND", default_value = "0.0.0.0:8080")]
        bind: String,
        /// Postgres URL (`DATABASE_URL`).
        #[arg(long, env = "DATABASE_URL")]
        database_url: String,
        /// AES-256-GCM credential key (hex), shared with stiglab so the
        /// portal can decrypt `webhook_secret_cipher` rows.
        #[arg(long, env = "ONSAGER_CREDENTIAL_KEY")]
        credential_key: Option<String>,
        /// Synodic gate URL (e.g. `http://synodic:3001`).
        #[arg(long, env = "SYNODIC_URL")]
        synodic_url: Option<String>,
        /// Optional GitHub PAT used for posting check runs / comments when
        /// installation-token signing isn't wired up. Prefer per-installation
        /// tokens when available.
        #[arg(long, env = "GITHUB_TOKEN")]
        github_token: Option<String>,
        /// Public origin under which portal-served routes are reached
        /// (e.g. `https://app.onsager.ai`). Used by the OAuth callback URL
        /// and the cookie `Secure` attribute.
        #[arg(long, env = "PORTAL_PUBLIC_URL")]
        public_url: Option<String>,
        /// GitHub OAuth client id (owner mode).
        #[arg(long, env = "GITHUB_CLIENT_ID")]
        github_client_id: Option<String>,
        /// GitHub OAuth client secret (owner mode).
        #[arg(long, env = "GITHUB_CLIENT_SECRET")]
        github_client_secret: Option<String>,
        /// Cross-environment SSO — owner-side state-envelope HMAC secret.
        #[arg(long, env = "SSO_STATE_SECRET")]
        sso_state_secret: Option<String>,
        /// Cross-environment SSO — back-channel bearer secret shared
        /// between owner and relying parties.
        #[arg(long, env = "SSO_EXCHANGE_SECRET")]
        sso_exchange_secret: Option<String>,
        /// Comma-separated allowlist of hosts the owner will redirect
        /// back to. Entries are `*.subdomain.example.com` (strict
        /// subdomain match) or `host.example.com` (exact match).
        #[arg(long, env = "SSO_RETURN_HOST_ALLOWLIST")]
        sso_return_host_allowlist: Option<String>,
        /// Cross-environment SSO — relying side. When set, `/api/auth/github`
        /// redirects here instead of talking to GitHub directly.
        #[arg(long, env = "SSO_AUTH_DOMAIN")]
        sso_auth_domain: Option<String>,
    },
    /// Backfill issues + PRs for an existing project.
    Backfill {
        #[arg(long, env = "DATABASE_URL")]
        database_url: String,
        #[arg(long)]
        project: String,
        #[arg(long, default_value = "recent")]
        strategy: String,
        #[arg(long, default_value_t = 100)]
        cap: usize,
        #[arg(long, env = "GITHUB_TOKEN")]
        github_token: Option<String>,
    },
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(run(cli))
}

async fn run(cli: Cli) -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "onsager_portal=info".into()),
        )
        .init();

    match cli.command {
        Command::Serve {
            bind,
            database_url,
            credential_key,
            synodic_url,
            github_token,
            public_url,
            github_client_id,
            github_client_secret,
            sso_state_secret,
            sso_exchange_secret,
            sso_return_host_allowlist,
            sso_auth_domain,
        } => {
            let allowlist = sso_return_host_allowlist
                .as_deref()
                .map(onsager_portal::sso::parse_host_allowlist)
                .unwrap_or_default();
            let cfg = onsager_portal::config::Config {
                bind: bind.clone(),
                database_url,
                credential_key,
                synodic_url,
                github_token,
                public_url,
                github_client_id: github_client_id.filter(|s| !s.is_empty()),
                github_client_secret: github_client_secret.filter(|s| !s.is_empty()),
                sso_state_secret: sso_state_secret.filter(|s| !s.is_empty()),
                sso_exchange_secret: sso_exchange_secret.filter(|s| !s.is_empty()),
                sso_return_host_allowlist: allowlist,
                sso_auth_domain: sso_auth_domain
                    .filter(|s| !s.is_empty())
                    .map(|s| s.trim_end_matches('/').to_string()),
            };
            tracing::info!(%bind, "onsager-portal: starting webhook server");
            onsager_portal::server::run(cfg).await
        }
        Command::Backfill {
            database_url,
            project,
            strategy,
            cap,
            github_token,
        } => {
            let strategy: onsager_portal::backfill::Strategy = strategy.parse()?;
            let pool = onsager_portal::db::connect(&database_url).await?;
            onsager_portal::migrate::run(&pool).await?;
            let store = onsager_spine::EventStore::connect(&database_url).await?;
            let report =
                onsager_portal::backfill::run(&pool, &store, &project, strategy, cap, github_token)
                    .await?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            Ok(())
        }
    }
}
