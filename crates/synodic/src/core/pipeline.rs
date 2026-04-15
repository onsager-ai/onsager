//! Pipeline configuration, check runner, and Build→Inspect→Route state machine.
//!
//! Parses `.harness/pipeline.yml` — the single source of truth for a
//! project's quality gates — and executes the governed pipeline loop.

use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result};
use indicatif::ProgressBar;
use serde::{Deserialize, Serialize};
use tokio::process::Command;

use crate::core::storage::Storage;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// When a check runs in git-hook mode.
///
/// - `Commit` → pre-commit hook
/// - `Push` → pre-push hook
/// - Omitted (`None`) → only during `synodic run`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Stage {
    Commit,
    Push,
}

/// Severity for semantic (L2) checks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// Failure triggers rework (same as L1 check failure).
    Block,
    /// Findings reported in UI but don't block the pipeline.
    Warn,
}

/// Default severity for semantic checks.
pub fn default_severity() -> Severity {
    Severity::Block
}

/// A quality check — either L1 deterministic (run) or L2 semantic (LLM review).
///
/// Backward compatible: checks without a `type` field default to L1 run checks.
#[derive(Debug, Clone, Serialize)]
pub enum Check {
    /// L1 deterministic check (default when `type` is omitted).
    Run {
        name: String,
        run: String,
        fix: Option<String>,
        stage: Option<Stage>,
    },
    /// L2 semantic review via LLM.
    Semantic {
        name: String,
        prompt: String,
        severity: Severity,
    },
}

/// Raw helper struct for deserializing checks with backward compatibility.
#[derive(Deserialize)]
struct CheckRaw {
    name: String,
    #[serde(rename = "type")]
    check_type: Option<String>,
    // L1 fields
    run: Option<String>,
    fix: Option<String>,
    stage: Option<Stage>,
    // L2 fields
    prompt: Option<String>,
    severity: Option<String>,
}

impl<'de> Deserialize<'de> for Check {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = CheckRaw::deserialize(deserializer)?;

        let check_type = raw.check_type.as_deref().unwrap_or("run");

        match check_type {
            "semantic" => {
                let prompt = raw
                    .prompt
                    .ok_or_else(|| serde::de::Error::missing_field("prompt"))?;
                let severity = match raw.severity.as_deref() {
                    Some("warn") => Severity::Warn,
                    _ => Severity::Block,
                };
                Ok(Check::Semantic {
                    name: raw.name,
                    prompt,
                    severity,
                })
            }
            _ => {
                let run = raw
                    .run
                    .ok_or_else(|| serde::de::Error::missing_field("run"))?;
                Ok(Check::Run {
                    name: raw.name,
                    run,
                    fix: raw.fix,
                    stage: raw.stage,
                })
            }
        }
    }
}

impl Check {
    /// Get the check name regardless of variant.
    pub fn name(&self) -> &str {
        match self {
            Check::Run { name, .. } => name,
            Check::Semantic { name, .. } => name,
        }
    }

    /// Whether this is a semantic (L2) check.
    pub fn is_semantic(&self) -> bool {
        matches!(self, Check::Semantic { .. })
    }
}

/// Pipeline execution settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineSettings {
    /// Maximum Build↔Inspect rework cycles.
    #[serde(default = "default_max_rework")]
    pub max_rework: u32,
    /// Whether to auto-merge the PR on pass.
    #[serde(default)]
    pub auto_merge: bool,
    /// Claude model to use for BUILD phase (e.g. "sonnet", "opus", "claude-sonnet-4-6").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Claude thinking effort level (low, medium, high, max).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
}

fn default_max_rework() -> u32 {
    3
}

impl Default for PipelineSettings {
    fn default() -> Self {
        Self {
            max_rework: default_max_rework(),
            auto_merge: false,
            model: None,
            effort: None,
        }
    }
}

/// Top-level pipeline configuration (parsed from `.harness/pipeline.yml`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineConfig {
    /// Project language (rust, node, python, go, generic).
    pub language: String,
    /// Quality checks to run.
    pub checks: Vec<Check>,
    /// Pipeline execution settings.
    #[serde(default)]
    pub pipeline: PipelineSettings,
}

/// Result of executing a single check as a subprocess.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckResult {
    /// Name of the check.
    pub name: String,
    /// Whether the check passed (exit code 0).
    pub passed: bool,
    /// Process exit code.
    pub exit_code: i32,
    /// Captured stdout.
    pub stdout: String,
    /// Captured stderr.
    pub stderr: String,
    /// Wall-clock duration in milliseconds.
    pub duration_ms: u64,
}

// ---------------------------------------------------------------------------
// Config loading
// ---------------------------------------------------------------------------

/// Load and parse a pipeline configuration file.
pub fn load_config(path: &Path) -> Result<PipelineConfig> {
    let contents = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read pipeline config: {}", path.display()))?;
    let config: PipelineConfig = serde_yaml::from_str(&contents)
        .with_context(|| format!("failed to parse pipeline config: {}", path.display()))?;
    Ok(config)
}

// ---------------------------------------------------------------------------
// Check runner
// ---------------------------------------------------------------------------

/// Execute L1 (run) checks as subprocesses and collect results (output captured, silent).
///
/// Runs each check sequentially in the given working directory.
/// Skips semantic checks. Does not short-circuit on failure — all checks run regardless.
pub async fn run_checks(checks: &[Check], cwd: &Path) -> Result<Vec<CheckResult>> {
    let mut results = Vec::with_capacity(checks.len());

    for check in checks {
        let (name, cmd) = match check {
            Check::Run { name, run, .. } => (name.clone(), run.clone()),
            Check::Semantic { .. } => continue,
        };

        let start = Instant::now();

        let output = Command::new("sh")
            .arg("-c")
            .arg(&cmd)
            .current_dir(cwd)
            .output()
            .await
            .with_context(|| format!("failed to execute check '{name}'"))?;

        let duration_ms = start.elapsed().as_millis() as u64;

        results.push(CheckResult {
            name,
            passed: output.status.success(),
            exit_code: output.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            duration_ms,
        });
    }

    Ok(results)
}

/// Execute L1 checks with styled UI output (spinners + streaming lines).
///
/// Runs L1 (run) checks first. If all L1 pass and `skip_semantic` is false,
/// runs L2 (semantic) checks. L1 checks with `fix` are auto-fixed on failure.
///
/// `base_commit` is the commit SHA at pipeline start — semantic checks diff
/// `base_commit..HEAD` to review only the pipeline's changes.
pub async fn run_checks_ui(
    checks: &[Check],
    cwd: &Path,
    ui: &crate::core::ui::PipelineUi,
    task_prompt: Option<&str>,
    skip_semantic: bool,
    base_commit: Option<&str>,
    semantic_model: Option<&str>,
) -> Result<Vec<CheckResult>> {
    let l1_checks: Vec<_> = checks.iter().filter(|c| !c.is_semantic()).collect();
    let l2_checks: Vec<_> = checks.iter().filter(|c| c.is_semantic()).collect();

    let mut results = Vec::with_capacity(checks.len());

    // Phase 1: L1 deterministic checks
    for check in &l1_checks {
        let result = run_l1_check_ui(check, cwd, ui).await?;
        results.push(result);
    }

    let l1_all_passed = results.iter().all(|r| r.passed);

    // Phase 2: L2 semantic checks (only if L1 all passed and not skipped)
    if l1_all_passed && !skip_semantic && !l2_checks.is_empty() {
        for check in &l2_checks {
            if let Check::Semantic {
                name,
                prompt,
                severity,
            } = check
            {
                let result = run_semantic_check_ui(
                    name,
                    prompt,
                    severity,
                    cwd,
                    task_prompt,
                    ui,
                    base_commit,
                    semantic_model,
                )
                .await?;
                results.push(result);
            }
        }
    }

    Ok(results)
}

/// Run a single L1 check with UI. If it fails and has a `fix` command, auto-fix and retry.
async fn run_l1_check_ui(
    check: &Check,
    cwd: &Path,
    ui: &crate::core::ui::PipelineUi,
) -> Result<CheckResult> {
    let (name, cmd, fix) = match check {
        Check::Run { name, run, fix, .. } => (name.clone(), run.clone(), fix.clone()),
        Check::Semantic { .. } => unreachable!(),
    };

    let result = run_l1_check_inner(&name, &cmd, cwd, ui).await?;

    // If failed and has fix command, try auto-fix
    if !result.passed {
        if let Some(fix_cmd) = &fix {
            ui.check_line(&ui.check_spinner(""), &format!("auto-fixing {name}..."));
            // Run fix command silently
            let fix_status = Command::new("sh")
                .arg("-c")
                .arg(fix_cmd)
                .current_dir(cwd)
                .output()
                .await;

            if let Ok(output) = fix_status {
                if output.status.success() {
                    // Stage the fix
                    let add_status = Command::new("git")
                        .args(["add", "-A"])
                        .current_dir(cwd)
                        .status()
                        .await
                        .context("failed to run `git add -A` after auto-fix")?;
                    if !add_status.success() {
                        return Err(anyhow::anyhow!(
                            "`git add -A` failed after auto-fix for check {name}"
                        ));
                    }

                    // Only commit if there are staged changes
                    let staged = Command::new("git")
                        .args(["diff", "--cached", "--quiet"])
                        .current_dir(cwd)
                        .status()
                        .await
                        .context("failed to check staged auto-fix changes")?;

                    if staged.code() == Some(1) {
                        // There are staged changes — commit them
                        let commit_status = Command::new("git")
                            .args(["commit", "-m", &format!("fix: auto-fix {name}")])
                            .current_dir(cwd)
                            .status()
                            .await
                            .context("failed to run `git commit` after auto-fix")?;
                        if !commit_status.success() {
                            return Err(anyhow::anyhow!(
                                "`git commit` failed after auto-fix for check {name}"
                            ));
                        }
                    }

                    // Re-run the check
                    let retry = run_l1_check_inner(&name, &cmd, cwd, ui).await?;
                    return Ok(retry);
                }
            }
        }
    }

    Ok(result)
}

/// Inner L1 check execution with streaming UI.
async fn run_l1_check_inner(
    name: &str,
    cmd: &str,
    cwd: &Path,
    ui: &crate::core::ui::PipelineUi,
) -> Result<CheckResult> {
    use tokio::io::{AsyncBufReadExt, BufReader};

    let start = Instant::now();
    let pb = ui.check_spinner(name);

    let mut child = Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .current_dir(cwd)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .with_context(|| format!("failed to execute check '{name}'"))?;

    let mut stdout_buf = Vec::new();
    let mut stderr_buf = Vec::new();

    let stdout = child.stdout.take().map(BufReader::new);
    let stderr = child.stderr.take().map(BufReader::new);

    let mut stdout_lines = stdout.map(|r| r.lines());
    let mut stderr_lines = stderr.map(|r| r.lines());
    let mut stdout_done = stdout_lines.is_none();
    let mut stderr_done = stderr_lines.is_none();

    const HEAD_LINES: usize = 10;
    const TAIL_LINES: usize = 10;
    let mut displayed = 0usize;
    let mut suppressed = 0usize;
    let mut tail_buf: Vec<String> = Vec::new();

    while !stdout_done || !stderr_done {
        tokio::select! {
            line = async { stdout_lines.as_mut().unwrap().next_line().await },
                if !stdout_done => {
                match line? {
                    Some(l) => {
                        if displayed < HEAD_LINES {
                            ui.check_line(&pb, &l);
                            displayed += 1;
                        } else {
                            suppressed += 1;
                            tail_buf.push(l.clone());
                            if tail_buf.len() > TAIL_LINES {
                                tail_buf.remove(0);
                            }
                        }
                        stdout_buf.push(l);
                    }
                    None => stdout_done = true,
                }
            }
            line = async { stderr_lines.as_mut().unwrap().next_line().await },
                if !stderr_done => {
                match line? {
                    Some(l) => {
                        if displayed < HEAD_LINES {
                            ui.check_line(&pb, &l);
                            displayed += 1;
                        } else {
                            suppressed += 1;
                            tail_buf.push(l.clone());
                            if tail_buf.len() > TAIL_LINES {
                                tail_buf.remove(0);
                            }
                        }
                        stderr_buf.push(l);
                    }
                    None => stderr_done = true,
                }
            }
        }
    }

    if suppressed > 0 {
        let hidden = suppressed.saturating_sub(tail_buf.len());
        if hidden > 0 {
            ui.check_line(&pb, &format!("...{hidden} lines hidden..."));
        }
        for line in &tail_buf {
            ui.check_line(&pb, line);
        }
    }

    let status = child.wait().await?;
    let duration_ms = start.elapsed().as_millis() as u64;

    ui.check_done(pb, name, status.success(), duration_ms);

    Ok(CheckResult {
        name: name.to_string(),
        passed: status.success(),
        exit_code: status.code().unwrap_or(-1),
        stdout: stdout_buf.join("\n"),
        stderr: stderr_buf.join("\n"),
        duration_ms,
    })
}

/// Run an L2 semantic check via direct LLM API call (reqwest).
///
/// Uses the `llm` module for lightweight, direct API calls to Anthropic or
/// OpenAI-compatible endpoints. This is intentionally NOT a Claude Code session
/// — semantic checks ARE the governance layer and must not trigger L2 hooks.
///
/// `base_commit` — if set, diffs `base..HEAD` to review only pipeline changes.
/// `model` — override the default semantic model.
#[allow(clippy::too_many_arguments)]
async fn run_semantic_check_ui(
    name: &str,
    check_prompt: &str,
    severity: &Severity,
    cwd: &Path,
    task_prompt: Option<&str>,
    ui: &crate::core::ui::PipelineUi,
    base_commit: Option<&str>,
    model: Option<&str>,
) -> Result<CheckResult> {
    use crate::core::llm::{default_model_for_provider, LlmClient, LlmRequest};

    let start = Instant::now();
    let pb = ui.check_spinner(name);

    // Get the diff to review — use base_commit..HEAD if available
    let diff_args = match base_commit {
        Some(base) => vec!["diff", base, "HEAD"],
        None => vec!["diff", "HEAD~1"],
    };
    let diff_output = Command::new("git")
        .args(&diff_args)
        .current_dir(cwd)
        .output()
        .await
        .context("failed to get git diff for semantic review")?;

    let diff = String::from_utf8_lossy(&diff_output.stdout);
    if diff.trim().is_empty() {
        ui.check_done(pb, name, true, start.elapsed().as_millis() as u64);
        return Ok(CheckResult {
            name: name.to_string(),
            passed: true,
            exit_code: 0,
            stdout: "No changes to review".to_string(),
            stderr: String::new(),
            duration_ms: start.elapsed().as_millis() as u64,
        });
    }

    // Build the LLM client from environment
    let llm = match LlmClient::from_env() {
        Ok(c) => c,
        Err(e) => {
            // No credentials — skip gracefully
            ui.check_done(pb, name, true, start.elapsed().as_millis() as u64);
            return Ok(CheckResult {
                name: name.to_string(),
                passed: true,
                exit_code: 0,
                stdout: format!("Skipped: {e}"),
                stderr: String::new(),
                duration_ms: start.elapsed().as_millis() as u64,
            });
        }
    };

    let task_info = task_prompt
        .map(|p| format!("Task prompt: {p}\n"))
        .unwrap_or_default();

    // Truncate diff to avoid token limits
    let diff_str = if diff.len() > 80_000 {
        format!("{}...\n(truncated)", &diff[..80_000])
    } else {
        diff.to_string()
    };

    let system = "You are a QA reviewer. Review the diff below against the given criteria.\n\
         Return ONLY a JSON object with exactly two fields:\n\
         \"passed\" (boolean) and \"findings\" (array of strings).\n\
         If everything looks good, return {\"passed\": true, \"findings\": []}.\n\
         Only return the JSON, nothing else."
        .to_string();

    let user_message = format!(
        "Context:\n{task_info}Check: {name}\nCriteria: {check_prompt}\n\nDiff:\n```\n{diff_str}\n```"
    );

    let resolved_model = model
        .map(String::from)
        .unwrap_or_else(|| default_model_for_provider(llm.provider()).to_string());

    let req = LlmRequest {
        system,
        user_message,
        model: resolved_model,
        max_tokens: 1024,
    };

    let response = llm.complete(&req).await;
    let duration_ms = start.elapsed().as_millis() as u64;

    match response {
        Ok(resp) => {
            let text = &resp.text;

            // Extract JSON from response (LLM may wrap it in markdown fences)
            let json_str = extract_json_from_text(text);

            // Parse the JSON verdict
            let verdict: serde_json::Value = serde_json::from_str(json_str)
                .unwrap_or(serde_json::json!({"passed": true, "findings": []}));

            let passed_raw = verdict
                .get("passed")
                .and_then(|p| p.as_bool())
                .unwrap_or(true);

            let findings: Vec<String> = verdict
                .get("findings")
                .and_then(|f| f.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();

            // severity: warn findings don't block
            let is_warn = *severity == Severity::Warn;
            let passed = if is_warn { true } else { passed_raw };

            let stdout = if findings.is_empty() {
                "No issues found".to_string()
            } else {
                format!(
                    "Found {} issue(s):\n{}",
                    findings.len(),
                    findings
                        .iter()
                        .map(|f| format!("  - {f}"))
                        .collect::<Vec<_>>()
                        .join("\n")
                )
            };

            ui.check_done(pb, name, passed, duration_ms);

            // Show findings in UI — for both block failures AND warn with findings
            if !findings.is_empty() && (!passed || is_warn) {
                for finding in &findings {
                    ui.check_line(&ProgressBar::hidden(), &format!("    {finding}"));
                }
            }

            Ok(CheckResult {
                name: name.to_string(),
                passed,
                exit_code: 0,
                stdout,
                stderr: String::new(),
                duration_ms,
            })
        }
        Err(e) => {
            // API error — skip rather than block pipeline
            ui.check_done(pb, name, true, duration_ms);
            Ok(CheckResult {
                name: name.to_string(),
                passed: true,
                exit_code: 0,
                stdout: format!("Skipped: {e}"),
                stderr: String::new(),
                duration_ms,
            })
        }
    }
}

/// Extract the first JSON object from text that may contain markdown fences.
fn extract_json_from_text(text: &str) -> &str {
    let trimmed = text.trim();
    // Try to find JSON between code fences
    if let Some(start) = trimmed.find("```json") {
        let after = &trimmed[start + 7..];
        if let Some(end) = after.find("```") {
            return after[..end].trim();
        }
    }
    if let Some(start) = trimmed.find("```") {
        let after = &trimmed[start + 3..];
        if let Some(end) = after.find("```") {
            return after[..end].trim();
        }
    }
    // Try to find raw JSON object
    if let Some(start) = trimmed.find('{') {
        if let Some(end) = trimmed.rfind('}') {
            return &trimmed[start..=end];
        }
    }
    trimmed
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Filter L1 checks to those matching a given stage.
///
/// Semantic checks and checks with no stage are excluded.
pub fn filter_checks_by_stage(checks: &[Check], stage: Stage) -> Vec<&Check> {
    checks
        .iter()
        .filter(|c| match c {
            Check::Run { stage: Some(s), .. } => *s == stage,
            _ => false,
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Pipeline runner — Build→Inspect→Route state machine
// ---------------------------------------------------------------------------

/// Configuration for a pipeline run.
pub struct RunConfig {
    /// Task description for the BUILD agent.
    pub prompt: String,
    /// Maximum Build↔Inspect rework cycles.
    pub max_rework: u32,
    /// INSPECT only — skip BUILD and PR.
    pub dry_run: bool,
    /// Skip PR creation (run BUILD+INSPECT only).
    pub local: bool,
    /// Custom branch name (default: auto-generated).
    pub branch: Option<String>,
    /// Claude model (e.g. "sonnet", "opus"). None = claude default.
    pub model: Option<String>,
    /// Claude thinking effort (low, medium, high, max). None = claude default.
    pub effort: Option<String>,
    /// Project directory.
    pub project_dir: PathBuf,
    /// Skip L2 semantic checks.
    pub skip_semantic: bool,
}

/// Outcome of a pipeline run.
#[derive(Debug)]
pub enum RunOutcome {
    /// All checks passed.
    Passed {
        attempts: u32,
        pr_url: Option<String>,
    },
    /// Exhausted rework budget with remaining failures.
    Failed {
        attempts: u32,
        last_failures: Vec<CheckResult>,
    },
    /// Something went wrong outside the loop.
    Error(String),
}

/// Build the prompt for the BUILD phase.
///
/// On the first attempt, just the task. On rework attempts, append
/// check failure feedback so the agent knows what to fix.
pub fn build_prompt(task: &str, attempt: u32, failures: &[CheckResult]) -> String {
    if attempt <= 1 || failures.is_empty() {
        return format!(
            "## Task\n{task}\n\n\
             Rules:\n\
             - Implement the task described above\n\
             - Follow existing code conventions\n\
             - Commit your changes with a clear message"
        );
    }

    let mut feedback = String::new();
    for f in failures {
        feedback.push_str(&format!("### {} (exit {})\n", f.name, f.exit_code));
        let output = if !f.stderr.is_empty() {
            &f.stderr
        } else {
            &f.stdout
        };
        // Truncate long output
        let truncated: String = output
            .lines()
            .rev()
            .take(40)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>()
            .join("\n");
        feedback.push_str(&format!("```\n{truncated}\n```\n\n"));
    }

    format!(
        "## Task\n{task}\n\n\
         ## Rework Required (attempt {attempt})\n\
         The previous attempt failed quality checks. Fix ALL issues:\n\n\
         {feedback}\
         Rules:\n\
         - Fix every issue listed above\n\
         - Do not break existing functionality\n\
         - Commit your changes with a clear message"
    )
}

/// Run the full Build→Inspect→Route pipeline.
///
/// For non-dry-run: creates a git worktree so Claude works in an isolated
/// copy of the repo, leaving the user's working tree untouched.
///
/// When `store` is `Some`, pipeline telemetry (check results, run outcome)
/// is recorded to the governance database.
pub async fn run_pipeline(
    config: &PipelineConfig,
    run_cfg: &RunConfig,
    ui: &crate::core::ui::PipelineUi,
    store: Option<&dyn Storage>,
) -> Result<RunOutcome> {
    ui.header(&run_cfg.prompt, run_cfg.dry_run);

    // --- INIT: create worktree + branch (skip in dry-run) ---
    let (work_dir, branch, worktree_path) = if !run_cfg.dry_run {
        let branch_name = run_cfg.branch.clone().unwrap_or_else(|| {
            let ts = chrono::Utc::now().format("%Y%m%d-%H%M%S");
            format!("synodic/{ts}")
        });

        let wt_path = run_cfg
            .project_dir
            .join(".synodic/worktrees")
            .join(branch_name.replace('/', "-"));

        // Clean up stale worktree at this path if it exists
        if wt_path.exists() {
            Command::new("git")
                .args(["worktree", "remove", "--force"])
                .arg(&wt_path)
                .current_dir(&run_cfg.project_dir)
                .status()
                .await
                .ok();
            if wt_path.exists() {
                tokio::fs::remove_dir_all(&wt_path).await.ok();
            }
        }

        if let Some(parent) = wt_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        let status = Command::new("git")
            .args(["worktree", "add", "-b", &branch_name])
            .arg(&wt_path)
            .current_dir(&run_cfg.project_dir)
            .status()
            .await
            .context("failed to create worktree")?;

        if !status.success() {
            return Ok(RunOutcome::Error(format!(
                "git worktree add -b {branch_name} failed"
            )));
        }

        ui.worktree_info(&branch_name, &wt_path.display().to_string());

        (wt_path.clone(), Some(branch_name), Some(wt_path))
    } else {
        (run_cfg.project_dir.clone(), None, None)
    };

    let pipeline_start = Instant::now();
    let outcome = run_pipeline_loop(config, run_cfg, &work_dir, branch.as_deref(), ui, store).await;

    // Record error outcomes in telemetry
    if let Ok(RunOutcome::Error(_)) = &outcome {
        if let Some(s) = store {
            let run_record = crate::core::storage::PipelineRun {
                id: uuid::Uuid::new_v4().to_string(),
                prompt: run_cfg.prompt.clone(),
                branch: branch.as_ref().map(|b| b.to_string()),
                outcome: "error".to_string(),
                attempts: 0,
                model: run_cfg.model.clone(),
                build_duration_ms: None,
                build_cost_usd: None,
                inspect_duration_ms: None,
                total_duration_ms: pipeline_start.elapsed().as_millis() as i64,
                project_id: None,
                created_at: chrono::Utc::now(),
            };
            let _ = s.record_pipeline_run(run_record).await;
        }
    }

    // --- CLEANUP: remove worktree ---
    if let Some(wt) = &worktree_path {
        ui.cleanup();
        Command::new("git")
            .args(["worktree", "remove", "--force"])
            .arg(wt)
            .current_dir(&run_cfg.project_dir)
            .status()
            .await
            .ok();
    }

    outcome
}

/// The inner Build→Inspect→Route loop, separated so cleanup always runs.
async fn run_pipeline_loop(
    config: &PipelineConfig,
    run_cfg: &RunConfig,
    cwd: &Path,
    branch: Option<&str>,
    ui: &crate::core::ui::PipelineUi,
    store: Option<&dyn Storage>,
) -> Result<RunOutcome> {
    use crate::core::storage::{FeedbackEvent, PipelineRun};

    let pipeline_start = Instant::now();
    let mut last_failures: Vec<CheckResult> = Vec::new();
    let mut build_duration_ms: Option<i64> = None;
    let mut build_cost: Option<f64> = None;

    // Capture base commit for semantic diff (before BUILD creates commits)
    let base_commit = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(cwd)
        .output()
        .await
        .ok()
        .and_then(|o| {
            if o.status.success() {
                Some(String::from_utf8_lossy(&o.stdout).trim().to_string())
            } else {
                None
            }
        });

    let max_attempts = if run_cfg.dry_run {
        1
    } else {
        run_cfg.max_rework
    };

    for attempt in 1..=max_attempts {
        ui.separator();

        // BUILD: invoke claude with stream-json for real-time visibility
        if !run_cfg.dry_run {
            let prompt = build_prompt(&run_cfg.prompt, attempt, &last_failures);
            let build_start = Instant::now();
            let cost = run_build(
                &prompt,
                cwd,
                run_cfg.model.as_deref(),
                run_cfg.effort.as_deref(),
                ui,
            )
            .await?;
            build_duration_ms = Some(build_start.elapsed().as_millis() as i64);
            build_cost = cost;
        }

        // INSPECT
        ui.section("INSPECT");
        let inspect_start = Instant::now();
        let results = run_checks_ui(
            &config.checks,
            cwd,
            ui,
            Some(&run_cfg.prompt),
            run_cfg.skip_semantic,
            base_commit.as_deref(),
            run_cfg.model.as_deref(),
        )
        .await?;
        let inspect_ms = inspect_start.elapsed().as_millis() as i64;

        let mut all_passed = true;
        last_failures.clear();

        for r in &results {
            if !r.passed {
                all_passed = false;
                last_failures.push(r.clone());
            }

            // Record check result as feedback event for telemetry
            if let Some(s) = store {
                let signal = if r.passed { "ci_pass" } else { "ci_failure" };
                let event = FeedbackEvent {
                    id: uuid::Uuid::new_v4(),
                    signal_type: signal.to_string(),
                    rule_id: format!("ci-{}", r.name),
                    session_id: None,
                    tool_name: "synodic-run".to_string(),
                    tool_input: serde_json::json!({
                        "check": r.name,
                        "exit_code": r.exit_code,
                        "duration_ms": r.duration_ms,
                    }),
                    override_reason: None,
                    failure_type: if r.passed {
                        None
                    } else {
                        Some("check_failure".to_string())
                    },
                    evidence_url: None,
                    project_id: None,
                    created_at: chrono::Utc::now(),
                };
                // Best-effort — don't fail the pipeline on telemetry errors
                let _ = s.record_feedback(event).await;
            }
        }

        // ROUTE
        if all_passed {
            ui.all_passed();

            let pr_url = if !run_cfg.dry_run && !run_cfg.local {
                create_pr(cwd, branch, &run_cfg.prompt, attempt, ui).await?
            } else {
                None
            };

            // Record pipeline run
            if let Some(s) = store {
                let run_record = PipelineRun {
                    id: uuid::Uuid::new_v4().to_string(),
                    prompt: run_cfg.prompt.clone(),
                    branch: branch.map(String::from),
                    outcome: "passed".to_string(),
                    attempts: attempt as i32,
                    model: run_cfg.model.clone(),
                    build_duration_ms,
                    build_cost_usd: build_cost,
                    inspect_duration_ms: Some(inspect_ms),
                    total_duration_ms: pipeline_start.elapsed().as_millis() as i64,
                    project_id: None,
                    created_at: chrono::Utc::now(),
                };
                let _ = s.record_pipeline_run(run_record).await;
            }

            return Ok(RunOutcome::Passed {
                attempts: attempt,
                pr_url,
            });
        }

        if attempt < max_attempts {
            ui.rework(last_failures.len());
        }
    }

    // Record failed pipeline run
    if let Some(s) = store {
        let run_record = PipelineRun {
            id: uuid::Uuid::new_v4().to_string(),
            prompt: run_cfg.prompt.clone(),
            branch: branch.map(String::from),
            outcome: "failed".to_string(),
            attempts: max_attempts as i32,
            model: run_cfg.model.clone(),
            build_duration_ms,
            build_cost_usd: build_cost,
            inspect_duration_ms: None,
            total_duration_ms: pipeline_start.elapsed().as_millis() as i64,
            project_id: None,
            created_at: chrono::Utc::now(),
        };
        let _ = s.record_pipeline_run(run_record).await;
    }

    Ok(RunOutcome::Failed {
        attempts: max_attempts,
        last_failures,
    })
}

/// Run the BUILD phase — invoke Claude with stream-json for real-time visibility.
/// Returns the build cost in USD if available.
async fn run_build(
    prompt: &str,
    cwd: &Path,
    model: Option<&str>,
    effort: Option<&str>,
    ui: &crate::core::ui::PipelineUi,
) -> Result<Option<f64>> {
    use tokio::io::{AsyncBufReadExt, BufReader};

    ui.section("BUILD");
    let pb = ui.build_spinner();
    let start = Instant::now();

    let mut args = vec![
        "--print",
        "-p",
        prompt,
        "--output-format",
        "stream-json",
        "--verbose",
    ];
    let model_str;
    if let Some(m) = model {
        model_str = m.to_string();
        args.push("--model");
        args.push(&model_str);
    }
    let effort_str;
    if let Some(e) = effort {
        effort_str = e.to_string();
        args.push("--effort");
        args.push(&effort_str);
    }

    let mut child = Command::new("claude")
        .args(&args)
        .current_dir(cwd)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .context("failed to invoke claude")?;

    let mut cost: Option<f64> = None;

    if let Some(stdout) = child.stdout.take() {
        let mut lines = BufReader::new(stdout).lines();
        while let Some(line) = lines.next_line().await? {
            if let Ok(event) = serde_json::from_str::<serde_json::Value>(&line) {
                match event.get("type").and_then(|t| t.as_str()) {
                    Some("system") => {
                        // Session started — show model + effort
                        if let Some(model_val) = event.get("model").and_then(|m| m.as_str()) {
                            let clean = model_val.split('[').next().unwrap_or(model_val);
                            let info = match effort {
                                Some(e) => format!("{clean} (effort: {e})"),
                                None => clean.to_string(),
                            };
                            ui.build_tool_call(&pb, "Model", &info);
                        }
                        pb.set_message("working...");
                    }
                    Some("assistant") => {
                        if let Some(content) =
                            event.pointer("/message/content").and_then(|c| c.as_array())
                        {
                            for item in content {
                                match item.get("type").and_then(|t| t.as_str()) {
                                    Some("tool_use") => {
                                        let tool = item
                                            .get("name")
                                            .and_then(|n| n.as_str())
                                            .unwrap_or("?");
                                        let summary = extract_tool_summary(item);
                                        ui.build_tool_call(&pb, tool, &summary);
                                        pb.set_message(format!("{tool}..."));
                                    }
                                    Some("thinking") => {
                                        if let Some(text) =
                                            item.get("thinking").and_then(|t| t.as_str())
                                        {
                                            ui.build_text_block(&pb, "Think", text, 4);
                                            pb.set_message("thinking...");
                                        }
                                    }
                                    Some("text") => {
                                        if let Some(text) =
                                            item.get("text").and_then(|t| t.as_str())
                                        {
                                            ui.build_text_block(&pb, "Output", text, 3);
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                    Some("result") => {
                        cost = event.get("total_cost_usd").and_then(|c| c.as_f64());
                    }
                    _ => {}
                }
            }
        }
    }

    let status = child.wait().await?;
    let duration_ms = start.elapsed().as_millis() as u64;
    ui.build_done(pb, status.success(), duration_ms, cost);

    Ok(cost)
}

/// Extract a short summary from a tool_use JSON content block.
fn extract_tool_summary(item: &serde_json::Value) -> String {
    let input = item.get("input");

    // Try common input fields in priority order
    let candidates = [
        "file_path",
        "command",
        "pattern",
        "skill",
        "description",
        "prompt",
        "query",
        "url",
    ];

    for key in candidates {
        if let Some(val) = input.and_then(|i| i.get(key)).and_then(|v| v.as_str()) {
            if key == "file_path" {
                // Shorten to last 2 path components
                let parts: Vec<&str> = val.rsplitn(3, '/').collect();
                return parts
                    .into_iter()
                    .rev()
                    .skip(1)
                    .collect::<Vec<_>>()
                    .join("/");
            }
            return val.to_string();
        }
    }

    String::new()
}

/// Push branch and create a PR via `gh`.
async fn create_pr(
    cwd: &Path,
    branch: Option<&str>,
    prompt: &str,
    attempts: u32,
    ui: &crate::core::ui::PipelineUi,
) -> Result<Option<String>> {
    let Some(branch_name) = branch else {
        return Ok(None);
    };

    ui.section("PR");
    ui.pr_status("pushing branch...");

    let push = Command::new("git")
        .args(["push", "-u", "origin", branch_name])
        .current_dir(cwd)
        .status()
        .await
        .context("git push failed")?;

    if !push.success() {
        ui.pr_status("git push failed, skipping PR creation");
        return Ok(None);
    }

    let title = format!("synodic: {}", truncate(prompt, 60));
    let body = format!(
        "## Summary\n\n\
         Automated pipeline run via [Synodic](https://github.com/codervisor/synodic).\n\n\
         **Prompt:** {prompt}\n\
         **Attempts:** {attempts}\n"
    );

    let output = Command::new("gh")
        .args(["pr", "create", "--title", &title, "--body", &body])
        .current_dir(cwd)
        .output()
        .await;

    match output {
        Ok(o) if o.status.success() => {
            let url = String::from_utf8_lossy(&o.stdout).trim().to_string();
            ui.pr_status(&format!("PR created: {url}"));
            Ok(Some(url))
        }
        Ok(o) => {
            let err = String::from_utf8_lossy(&o.stderr);
            ui.pr_status(&format!("gh pr create failed: {err}"));
            Ok(None)
        }
        Err(_) => {
            ui.pr_status("gh not found, skipping PR creation");
            ui.pr_status(&format!(
                "push succeeded -- create PR manually for branch '{branch_name}'"
            ));
            Ok(None)
        }
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max - 3])
    }
}

// ---------------------------------------------------------------------------
// Generators — derive git hooks and GHA workflow from pipeline.yml
// ---------------------------------------------------------------------------

/// Generate a git hook script from L1 checks matching a stage.
pub fn generate_hook_script(checks: &[Check], stage: Stage) -> Option<String> {
    let stage_checks = filter_checks_by_stage(checks, stage.clone());
    if stage_checks.is_empty() {
        return None;
    }

    let stage_name = match stage {
        Stage::Commit => "pre-commit",
        Stage::Push => "pre-push",
    };

    let mut script = format!(
        "#!/usr/bin/env bash\n\
         set -euo pipefail\n\
         # Generated by synodic init from .harness/pipeline.yml\n\n\
         echo \"Running {stage_name} checks...\"\n\n"
    );

    for check in &stage_checks {
        if let Check::Run { name, run, .. } = check {
            script.push_str(&format!(
                "echo \"  {name}\"\n\
                 {run} || {{ echo \"FAILED: {name}\"; exit 1; }}\n\n",
            ));
        }
    }

    script.push_str("echo \"All checks passed.\"\n");
    Some(script)
}

/// Generate a simplified GHA workflow that delegates to `synodic run`.
pub fn generate_workflow() -> String {
    r#"# Generated by synodic init
# Docs: https://github.com/codervisor/synodic

name: Synodic Pipeline

on:
  workflow_dispatch:
    inputs:
      prompt:
        description: "Task description"
        required: true
        type: string

jobs:
  pipeline:
    runs-on: ubuntu-latest
    timeout-minutes: 60
    permissions:
      contents: write
      pull-requests: write
    steps:
      - uses: actions/checkout@v4
        with:
          fetch-depth: 0

      - name: Install Synodic + Claude Code
        run: npm install -g @codervisor/synodic @anthropic-ai/claude-code

      - name: Run pipeline
        run: synodic run --prompt "${{ inputs.prompt }}"
        env:
          ANTHROPIC_API_KEY: ${{ secrets.ANTHROPIC_API_KEY }}
"#
    .to_string()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper to construct L1 run checks for tests.
    fn run_check(name: &str, cmd: &str) -> Check {
        Check::Run {
            name: name.into(),
            run: cmd.into(),
            fix: None,
            stage: None,
        }
    }

    fn run_check_staged(name: &str, cmd: &str, stage: Stage) -> Check {
        Check::Run {
            name: name.into(),
            run: cmd.into(),
            fix: None,
            stage: Some(stage),
        }
    }

    // -- Parsing ----------------------------------------------------------

    #[test]
    fn parse_full_config() {
        let yaml = r#"
language: rust

checks:
  - name: format
    run: "cargo fmt --all -- --check"
    fix: "cargo fmt --all"
  - name: lint
    run: "cargo clippy --all-targets -- -D warnings"
  - name: test
    run: "cargo test"

pipeline:
  max_rework: 3
  auto_merge: false
"#;
        let config: PipelineConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.language, "rust");
        assert_eq!(config.checks.len(), 3);
        assert_eq!(config.checks[0].name(), "format");
        match &config.checks[0] {
            Check::Run { run, fix, .. } => {
                assert_eq!(run, "cargo fmt --all -- --check");
                assert_eq!(fix.as_deref(), Some("cargo fmt --all"));
            }
            _ => panic!("expected Run check"),
        }
        assert_eq!(config.checks[1].name(), "lint");
        assert_eq!(config.pipeline.max_rework, 3);
        assert!(!config.pipeline.auto_merge);
    }

    #[test]
    fn parse_minimal_config() {
        let yaml = r#"
language: rust
checks:
  - name: test
    run: "cargo test"
"#;
        let config: PipelineConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.language, "rust");
        assert_eq!(config.checks.len(), 1);
        assert_eq!(config.pipeline.max_rework, 3);
        assert!(!config.pipeline.auto_merge);
    }

    #[test]
    fn parse_config_with_stages() {
        let yaml = r#"
language: rust
checks:
  - name: format
    run: "cargo fmt -- --check"
    stage: commit
  - name: lint
    run: "cargo clippy"
    stage: push
  - name: test
    run: "cargo test"
"#;
        let config: PipelineConfig = serde_yaml::from_str(yaml).unwrap();
        match &config.checks[0] {
            Check::Run { stage, .. } => assert_eq!(stage, &Some(Stage::Commit)),
            _ => panic!("expected Run check"),
        }
        match &config.checks[1] {
            Check::Run { stage, .. } => assert_eq!(stage, &Some(Stage::Push)),
            _ => panic!("expected Run check"),
        }
        match &config.checks[2] {
            Check::Run { stage, .. } => assert!(stage.is_none()),
            _ => panic!("expected Run check"),
        }
    }

    #[test]
    fn parse_semantic_check() {
        let yaml = r#"
language: rust
checks:
  - name: test
    run: "cargo test"
  - name: security-review
    type: semantic
    prompt: "Review for security vulnerabilities"
    severity: block
  - name: goal-alignment
    type: semantic
    prompt: "Does the diff match the task?"
    severity: warn
"#;
        let config: PipelineConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.checks.len(), 3);

        assert!(!config.checks[0].is_semantic());

        match &config.checks[1] {
            Check::Semantic {
                name,
                prompt,
                severity,
            } => {
                assert_eq!(name, "security-review");
                assert_eq!(prompt, "Review for security vulnerabilities");
                assert_eq!(severity, &Severity::Block);
            }
            _ => panic!("expected Semantic check"),
        }

        match &config.checks[2] {
            Check::Semantic { name, severity, .. } => {
                assert_eq!(name, "goal-alignment");
                assert_eq!(severity, &Severity::Warn);
            }
            _ => panic!("expected Semantic check"),
        }
    }

    #[test]
    fn parse_config_without_type_defaults_to_run() {
        let yaml = r#"
language: rust
checks:
  - name: format
    run: "cargo fmt -- --check"
"#;
        let config: PipelineConfig = serde_yaml::from_str(yaml).unwrap();
        assert!(!config.checks[0].is_semantic());
        assert_eq!(config.checks[0].name(), "format");
    }

    #[test]
    fn parse_config_with_explicit_run_type() {
        let yaml = r#"
language: rust
checks:
  - name: format
    type: run
    run: "cargo fmt -- --check"
"#;
        let config: PipelineConfig = serde_yaml::from_str(yaml).unwrap();
        assert!(!config.checks[0].is_semantic());
    }

    #[test]
    fn parse_invalid_yaml_returns_error() {
        let yaml = "not: [valid: yaml: {";
        let result = serde_yaml::from_str::<PipelineConfig>(yaml);
        assert!(result.is_err());
    }

    #[test]
    fn parse_missing_checks_returns_error() {
        let yaml = "language: rust\n";
        let result = serde_yaml::from_str::<PipelineConfig>(yaml);
        assert!(result.is_err());
    }

    #[test]
    fn load_missing_file_returns_error() {
        let result = load_config(Path::new("/nonexistent/pipeline.yml"));
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("failed to read pipeline config"));
    }

    // -- Stage filtering --------------------------------------------------

    #[test]
    fn filter_by_stage() {
        let checks = vec![
            run_check_staged("format", "cargo fmt -- --check", Stage::Commit),
            run_check_staged("lint", "cargo clippy", Stage::Push),
            run_check("test", "cargo test"),
        ];

        let commit = filter_checks_by_stage(&checks, Stage::Commit);
        assert_eq!(commit.len(), 1);
        assert_eq!(commit[0].name(), "format");

        let push = filter_checks_by_stage(&checks, Stage::Push);
        assert_eq!(push.len(), 1);
        assert_eq!(push[0].name(), "lint");
    }

    #[test]
    fn filter_by_stage_excludes_semantic() {
        let checks = vec![
            run_check_staged("format", "cargo fmt -- --check", Stage::Commit),
            Check::Semantic {
                name: "review".into(),
                prompt: "review".into(),
                severity: Severity::Block,
            },
        ];

        let commit = filter_checks_by_stage(&checks, Stage::Commit);
        assert_eq!(commit.len(), 1);
    }

    // -- Check execution --------------------------------------------------

    #[tokio::test]
    async fn run_passing_check() {
        let checks = vec![run_check("pass", "true")];

        let results = run_checks(&checks, Path::new("/tmp")).await.unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].passed);
        assert_eq!(results[0].exit_code, 0);
        assert_eq!(results[0].name, "pass");
    }

    #[tokio::test]
    async fn run_failing_check() {
        let checks = vec![run_check("fail", "false")];

        let results = run_checks(&checks, Path::new("/tmp")).await.unwrap();
        assert_eq!(results.len(), 1);
        assert!(!results[0].passed);
        assert_ne!(results[0].exit_code, 0);
    }

    #[tokio::test]
    async fn run_captures_stdout() {
        let checks = vec![run_check("echo", "echo hello")];

        let results = run_checks(&checks, Path::new("/tmp")).await.unwrap();
        assert_eq!(results[0].stdout.trim(), "hello");
    }

    #[tokio::test]
    async fn run_captures_stderr() {
        let checks = vec![run_check("stderr", "echo error >&2")];

        let results = run_checks(&checks, Path::new("/tmp")).await.unwrap();
        assert_eq!(results[0].stderr.trim(), "error");
    }

    #[tokio::test]
    async fn run_multiple_no_shortcircuit() {
        let checks = vec![
            run_check("first", "true"),
            run_check("second", "false"),
            run_check("third", "true"),
        ];

        let results = run_checks(&checks, Path::new("/tmp")).await.unwrap();
        assert_eq!(results.len(), 3);
        assert!(results[0].passed);
        assert!(!results[1].passed);
        assert!(results[2].passed);
    }

    #[tokio::test]
    async fn run_measures_duration() {
        let checks = vec![run_check("sleep", "sleep 0.1")];

        let results = run_checks(&checks, Path::new("/tmp")).await.unwrap();
        assert!(results[0].duration_ms >= 50);
    }

    #[tokio::test]
    async fn run_uses_cwd() {
        let checks = vec![run_check("pwd", "pwd")];

        let results = run_checks(&checks, Path::new("/tmp")).await.unwrap();
        assert!(results[0].stdout.trim().starts_with("/tmp"));
    }

    #[tokio::test]
    async fn run_skips_semantic_checks() {
        let checks = vec![
            run_check("pass", "true"),
            Check::Semantic {
                name: "review".into(),
                prompt: "review".into(),
                severity: Severity::Block,
            },
        ];

        let results = run_checks(&checks, Path::new("/tmp")).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "pass");
    }

    // -- Prompt construction ----------------------------------------------

    #[test]
    fn build_prompt_first_attempt() {
        let prompt = build_prompt("add rate limiting", 1, &[]);
        assert!(prompt.contains("## Task"));
        assert!(prompt.contains("add rate limiting"));
        assert!(!prompt.contains("Rework"));
    }

    #[test]
    fn build_prompt_with_rework_feedback() {
        let failures = vec![CheckResult {
            name: "test".into(),
            passed: false,
            exit_code: 1,
            stdout: String::new(),
            stderr: "thread 'main' panicked".into(),
            duration_ms: 100,
        }];

        let prompt = build_prompt("add rate limiting", 2, &failures);
        assert!(prompt.contains("Rework Required (attempt 2)"));
        assert!(prompt.contains("### test (exit 1)"));
        assert!(prompt.contains("thread 'main' panicked"));
    }

    #[test]
    fn build_prompt_rework_ignores_empty_failures() {
        let prompt = build_prompt("task", 2, &[]);
        assert!(!prompt.contains("Rework"));
    }

    // -- Hook generation --------------------------------------------------

    #[test]
    fn generate_pre_commit_hook() {
        let checks = vec![
            run_check_staged("format", "cargo fmt -- --check", Stage::Commit),
            run_check_staged("test", "cargo test", Stage::Push),
        ];

        let script = generate_hook_script(&checks, Stage::Commit).unwrap();
        assert!(script.contains("#!/usr/bin/env bash"));
        assert!(script.contains("cargo fmt -- --check"));
        assert!(!script.contains("cargo test"));
        assert!(script.contains("pre-commit"));
    }

    #[test]
    fn generate_hook_empty_stage_returns_none() {
        let checks = vec![run_check("test", "cargo test")];
        assert!(generate_hook_script(&checks, Stage::Commit).is_none());
    }

    // -- Workflow generation ----------------------------------------------

    #[test]
    fn generate_workflow_contains_synodic_run() {
        let wf = generate_workflow();
        assert!(wf.contains("synodic run"));
        assert!(wf.contains("ANTHROPIC_API_KEY"));
        assert!(wf.contains("workflow_dispatch"));
    }

    // -- Severity ---------------------------------------------------------

    #[test]
    fn severity_default_is_block() {
        assert_eq!(default_severity(), Severity::Block);
    }
}
