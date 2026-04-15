//! ising — CLI entry point for the Ising continuous improvement engine.

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "ising",
    about = "Onsager Ising — continuous improvement engine"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Start the Ising observation loop
    Serve {
        /// Database URL for the event spine
        #[arg(long, env = "DATABASE_URL")]
        database_url: String,

        /// Analyzer tick interval in milliseconds
        #[arg(long, default_value = "5000")]
        tick_ms: u64,
    },

    /// Show current Ising status
    Status,
}

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Commands::Serve {
            database_url,
            tick_ms,
        } => {
            ising::cmd::serve::run(&database_url, tick_ms);
        }
        Commands::Status => {
            println!("ising: not running (status check requires a running instance)");
        }
    }
}
