use anyhow::Result;
use clap::Args;
use std::path::PathBuf;

use crate::core::pipeline::{self, RunConfig, RunOutcome};
use crate::core::storage;
use crate::core::ui::PipelineUi;

use crate::util;

/// Run the Build→Inspect→PR pipeline.
///
/// Reads `.harness/pipeline.yml`, invokes Claude Code for the BUILD phase,
/// runs quality checks (INSPECT), and creates a PR on success.
#[derive(Args)]
pub struct RunCmd {
    /// Task description (what to build, fix, or refactor)
    #[arg(long)]
    prompt: String,

    /// Max rework cycles (overrides pipeline.yml)
    #[arg(long)]
    max_rework: Option<u32>,

    /// Auto-merge PR on pass
    #[arg(long)]
    auto_merge: bool,

    /// Custom branch name
    #[arg(long)]
    branch: Option<String>,

    /// Claude model (e.g. "sonnet", "opus", "claude-sonnet-4-6")
    #[arg(long)]
    model: Option<String>,

    /// Claude thinking effort level (low, medium, high, max)
    #[arg(long)]
    effort: Option<String>,

    /// Run INSPECT only (skip BUILD + PR)
    #[arg(long)]
    dry_run: bool,

    /// Skip PR creation (run BUILD+INSPECT only)
    #[arg(long)]
    local: bool,

    /// Skip L2 semantic checks (faster, no API cost)
    #[arg(long)]
    no_semantic: bool,

    /// Project directory (default: repo root)
    #[arg(long)]
    dir: Option<String>,
}

impl RunCmd {
    pub async fn run(self) -> Result<()> {
        let root = match self.dir {
            Some(d) => PathBuf::from(d),
            None => util::find_repo_root()?,
        };

        // Load pipeline config
        let config_path = root.join(".harness/pipeline.yml");
        let config = pipeline::load_config(&config_path).map_err(|e| {
            anyhow::anyhow!("{e}\n\nRun `synodic init` to generate .harness/pipeline.yml")
        })?;

        let max_rework = self.max_rework.unwrap_or(config.pipeline.max_rework);

        let model = self.model.or(config.pipeline.model.clone());
        let effort = self.effort.or(config.pipeline.effort.clone());

        let run_cfg = RunConfig {
            prompt: self.prompt.clone(),
            max_rework,
            dry_run: self.dry_run,
            local: self.local,
            branch: self.branch,
            model,
            effort,
            project_dir: root,
            skip_semantic: self.no_semantic,
        };

        // Open storage for telemetry (best-effort — runs fine without DB)
        let db_url = storage::pool::resolve_database_url();
        let store = storage::pool::create_storage(&db_url).await.ok();

        let ui = PipelineUi::new();
        let outcome = pipeline::run_pipeline(&config, &run_cfg, &ui, store.as_deref()).await?;

        match &outcome {
            RunOutcome::Passed { pr_url, .. } => {
                ui.pipeline_passed(pr_url.as_deref());
                if let Some(url) = pr_url {
                    println!("{url}");
                }
                Ok(())
            }
            RunOutcome::Failed { last_failures, .. } => {
                ui.pipeline_failed(last_failures);
                std::process::exit(1);
            }
            RunOutcome::Error(msg) => {
                ui.error(msg);
                std::process::exit(1);
            }
        }
    }
}
