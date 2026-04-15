use clap::Parser;
use synodic::cmd;

/// Synodic — AI agent governance and orchestration
#[derive(Parser)]
#[command(
    name = "synodic",
    version,
    about = "The tool that watches the AI agents.\n\nSetup:   synodic init\nRun:     synodic run --prompt \"...\"\nMonitor: synodic status\nManage:  synodic rules <list|show|promote|probe|optimize|...>"
)]
enum Cli {
    /// Setup governance + orchestration (hooks, pipeline, config)
    Init(cmd::init::InitCmd),

    /// Run the Build→Inspect→PR pipeline
    Run(cmd::run::RunCmd),

    /// Show governance health (safety, friction, coverage scores)
    Status(cmd::status::StatusCmd),

    /// Manage governance rules (list, show, promote, probe, optimize)
    Rules(cmd::rules::RulesCmd),

    /// Run the web server (dashboard + API)
    Serve(cmd::serve::ServeCmd),

    // ── Internal (called by hooks/automation, hidden from help) ──
    /// Evaluate tool call against rules (called by L2 hooks)
    #[command(hide = true)]
    Intercept(cmd::intercept::InterceptCmd),

    /// Record governance feedback (called by hooks/automation)
    #[command(hide = true)]
    Feedback(cmd::feedback::FeedbackCmd),

    // ── Deprecated aliases (subsumed by init/rules, kept for compat) ──
    /// Use `synodic init` instead
    #[command(hide = true)]
    Orchestrate(cmd::orchestrate::OrchestrationCmd),

    /// Use `synodic rules promote|crystallize|deprecate|check` instead
    #[command(hide = true)]
    Lifecycle(cmd::lifecycle::LifecycleCmd),

    /// Use `synodic rules probe` instead
    #[command(hide = true)]
    Probe(cmd::probe::ProbeCmd),

    /// Use `synodic rules optimize` instead
    #[command(hide = true)]
    Optimize(cmd::optimize::OptimizeCmd),
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli {
        Cli::Init(cmd) => cmd.run(),
        Cli::Run(cmd) => cmd.run().await,
        Cli::Status(cmd) => cmd.run().await,
        Cli::Rules(cmd) => cmd.run().await,
        Cli::Serve(cmd) => cmd.run().await,
        Cli::Intercept(cmd) => cmd.run(),
        Cli::Feedback(cmd) => cmd.run().await,
        Cli::Orchestrate(cmd) => cmd.run(),
        Cli::Lifecycle(cmd) => cmd.run().await,
        Cli::Probe(cmd) => cmd.run().await,
        Cli::Optimize(cmd) => cmd.run().await,
    }
}
