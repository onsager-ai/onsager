//! `refract` CLI — Submit intents to the Refract decomposer and emit the
//! lifecycle events onto the Onsager event spine (issue #35).

use anyhow::Context;
use clap::{Parser, Subcommand};
use refract::{DecomposerRegistry, Intent, Refract};

#[derive(Parser)]
#[command(name = "refract", about = "Intent decomposer for Onsager")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Submit an intent, decompose it, and print the resulting artifact
    /// ids. If `DATABASE_URL` is set, the full event lifecycle is also
    /// emitted on the spine; otherwise only the stdout report runs.
    Submit {
        /// Intent class — routes to the matching decomposer.
        #[arg(long)]
        class: String,
        /// Free-form description.
        #[arg(long)]
        description: String,
        /// Submitter identity.
        #[arg(long, default_value = "cli")]
        submitter: String,
        /// JSON payload (forwarded to the decomposer).
        #[arg(long, default_value = "{}")]
        payload: String,
    },
    /// List registered decomposers.
    List,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("refract=info")
        .init();

    let cli = Cli::parse();
    let registry = default_registry();

    match cli.cmd {
        Cmd::List => {
            for name in registry.names() {
                println!("{name}");
            }
            Ok(())
        }
        Cmd::Submit {
            class,
            description,
            submitter,
            payload,
        } => {
            let payload_json: serde_json::Value = serde_json::from_str(&payload)
                .with_context(|| format!("payload is not valid JSON: {payload}"))?;
            let intent = Intent::new(class, description, submitter, payload_json);

            let spine = match std::env::var("DATABASE_URL") {
                Ok(url) => match onsager_spine::EventStore::connect(&url).await {
                    Ok(s) => Some(s),
                    Err(e) => {
                        tracing::warn!("refract: spine connection failed ({e}); running offline");
                        None
                    }
                },
                Err(_) => {
                    tracing::info!("refract: DATABASE_URL unset; running offline");
                    None
                }
            };

            let rt = Refract::new(registry, spine);
            match rt.submit(&intent).await {
                Ok(result) => {
                    println!(
                        "intent {} decomposed into {} artifact(s):",
                        intent.id,
                        result.artifacts.len()
                    );
                    for a in &result.artifacts {
                        println!("  {} ({})", a.artifact_id, a.name);
                    }
                    Ok(())
                }
                Err(e) => {
                    eprintln!("intent {} failed: {e}", intent.id);
                    std::process::exit(1);
                }
            }
        }
    }
}

fn default_registry() -> DecomposerRegistry {
    let mut r = DecomposerRegistry::new();
    r.register(refract::decomposer::FileMigrationDecomposer);
    r
}
