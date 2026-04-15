//! forge — CLI entry point for the Forge production line subsystem.

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "forge", about = "Onsager Forge — production line subsystem")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Start the Forge scheduling loop
    Serve {
        /// Database URL for the event spine
        #[arg(long, env = "DATABASE_URL")]
        database_url: String,

        /// Tick interval in milliseconds
        #[arg(long, default_value = "1000")]
        tick_ms: u64,
    },

    /// Show current Forge status
    Status,
}

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Commands::Serve {
            database_url,
            tick_ms,
        } => {
            forge::cmd::serve::run(&database_url, tick_ms);
        }
        Commands::Status => {
            println!("forge: not running (status check requires a running instance)");
        }
    }
}
