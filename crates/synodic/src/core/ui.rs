//! Terminal UI for `synodic run` — spinners, colors, structured output.
//!
//! Provides `PipelineUi` which wraps all styled output. Auto-detects TTY
//! and falls back to plain text when piped or in CI.

use std::time::Duration;

use chrono::Local;
use console::{style, Term};
use indicatif::{ProgressBar, ProgressStyle};

use crate::core::pipeline::CheckResult;

/// Styled terminal UI for pipeline runs.
///
/// All output goes to stderr (matching convention — stdout is reserved
/// for machine-readable output like PR URLs).
pub struct PipelineUi {
    term: Term,
    is_tty: bool,
}

impl Default for PipelineUi {
    fn default() -> Self {
        Self::new()
    }
}

impl PipelineUi {
    pub fn new() -> Self {
        let term = Term::stderr();
        let is_tty = term.is_term();
        Self { term, is_tty }
    }

    // -- Pipeline chrome --------------------------------------------------

    pub fn header(&self, prompt: &str, dry_run: bool) {
        if dry_run {
            self.write(&format!(
                "\n {} Inspect only (dry run)\n",
                style("synodic").cyan().bold()
            ));
        } else {
            self.write(&format!(
                "\n {} Build->Inspect->PR pipeline\n",
                style("synodic").cyan().bold()
            ));
        }
        self.write(&format!("  prompt: {}", style(prompt).dim()));
        self.write("");
    }

    pub fn worktree_info(&self, branch: &str, path: &str) {
        self.write(&format!("  branch: {}", style(branch).dim()));
        self.write(&format!("  worktree: {}", style(path).dim()));
    }

    pub fn separator(&self) {
        let sep = "\u{2501}".repeat(52);
        self.write(&format!("\n{}", style(sep).dim()));
    }

    pub fn section(&self, name: &str) {
        self.write_ts(&format!("{}", style(name).bold()));
    }

    // -- BUILD phase ------------------------------------------------------

    pub fn build_spinner(&self) -> ProgressBar {
        self.make_spinner("invoking claude...")
    }

    pub fn build_tool_call(&self, pb: &ProgressBar, tool: &str, summary: &str) {
        let msg = format!(
            "  {} {}: {}",
            style("\u{2192}").cyan().dim(),
            tool,
            style(summary).dim()
        );
        if self.is_tty {
            pb.suspend(|| {
                let _ = self.term.write_line(&msg);
            });
        } else {
            self.write(&msg);
        }
    }

    /// Show a labeled block of text (for Think/Output), up to max_lines lines.
    pub fn build_text_block(&self, pb: &ProgressBar, label: &str, text: &str, max_lines: usize) {
        let all_lines: Vec<&str> = text
            .lines()
            .map(|l| l.trim())
            .filter(|l| !l.is_empty())
            .collect();

        if all_lines.is_empty() {
            return;
        }

        let shown = &all_lines[..all_lines.len().min(max_lines)];
        let remaining = all_lines.len().saturating_sub(max_lines);

        let header = format!(
            "  {} {}: {}",
            style("\u{2192}").cyan().dim(),
            label,
            style(shown[0]).dim()
        );

        if self.is_tty {
            pb.suspend(|| {
                let _ = self.term.write_line(&header);
                for line in &shown[1..] {
                    let _ = self.term.write_line(&format!("    {}", style(line).dim()));
                }
                if remaining > 0 {
                    let _ = self.term.write_line(&format!(
                        "    {}",
                        style(format!("...{remaining} more lines")).dim()
                    ));
                }
            });
        } else {
            self.write(&header);
            for line in &shown[1..] {
                self.write(&format!("    {}", style(line).dim()));
            }
            if remaining > 0 {
                self.write(&format!(
                    "    {}",
                    style(format!("...{remaining} more lines")).dim()
                ));
            }
        }
    }

    pub fn build_done(&self, pb: ProgressBar, success: bool, duration_ms: u64, cost: Option<f64>) {
        pb.finish_and_clear();
        let dur = format_duration(duration_ms);
        let cost_str = cost.map(|c| format!(", ${:.2}", c)).unwrap_or_default();

        if success {
            self.write(&format!(
                "  {} build complete {}",
                style("\u{2713}").green().bold(),
                style(format!("({dur}{cost_str})")).dim()
            ));
        } else {
            self.write(&format!(
                "  {} build failed {}",
                style("\u{2717}").red().bold(),
                style(format!("({dur})")).dim()
            ));
        }
    }

    // -- INSPECT phase ----------------------------------------------------

    pub fn check_spinner(&self, name: &str) -> ProgressBar {
        self.make_spinner(&format!("{name}..."))
    }

    pub fn check_line(&self, pb: &ProgressBar, line: &str) {
        let msg = format!("      {}", style(line).dim());
        if self.is_tty {
            pb.suspend(|| {
                let _ = self.term.write_line(&msg);
            });
        } else {
            self.write(&msg);
        }
    }

    pub fn check_done(&self, pb: ProgressBar, name: &str, passed: bool, duration_ms: u64) {
        pb.finish_and_clear();
        let dur = format_duration(duration_ms);
        let icon = if passed {
            style("\u{2713}").green().bold()
        } else {
            style("\u{2717}").red().bold()
        };
        self.write(&format!(
            "  {icon} {name} {}",
            style(format!("({dur})")).dim()
        ));
    }

    // -- Route / summary --------------------------------------------------

    pub fn rework(&self, n: usize) {
        self.write(&format!(
            "\n  {} check(s) failed {} reworking...",
            n,
            style("\u{2014}").dim()
        ));
    }

    pub fn all_passed(&self) {
        self.write_ts("All checks passed.");
    }

    pub fn pipeline_passed(&self, pr_url: Option<&str>) {
        let sep = "\u{2501}".repeat(52);
        self.write(&format!("{}", style(sep).dim()));
        self.write_ts(&format!(
            "{} {}",
            style("\u{2713}").green().bold(),
            style("Pipeline PASSED").green().bold(),
        ));
        if let Some(url) = pr_url {
            self.write(&format!("         PR: {}", style(url).underlined()));
        }
        self.write("");
    }

    pub fn pipeline_failed(&self, failures: &[CheckResult]) {
        let sep = "\u{2501}".repeat(52);
        self.write(&format!("{}", style(sep).dim()));
        self.write_ts(&format!(
            "{} {}",
            style("\u{2717}").red().bold(),
            style("Pipeline FAILED").red().bold(),
        ));
        self.write("");

        for f in failures {
            self.write(&format!(
                "  {} {} (exit {})",
                style("\u{2717}").red(),
                style(&f.name).bold(),
                f.exit_code
            ));

            // Show last 20 lines of the failing output
            let output = if !f.stderr.is_empty() {
                &f.stderr
            } else {
                &f.stdout
            };
            let lines: Vec<&str> = output.lines().collect();
            let start = lines.len().saturating_sub(20);
            if start > 0 {
                self.write(&format!(
                    "    {}",
                    style(format!("...{start} lines above")).dim()
                ));
            }
            for line in &lines[start..] {
                self.write(&format!("    {}", style(line).dim()));
            }
            self.write("");
        }
    }

    pub fn pr_status(&self, msg: &str) {
        self.write(&format!("  {}", msg));
    }

    pub fn cleanup(&self) {
        self.write(&format!("  {}", style("cleaning up worktree...").dim()));
    }

    pub fn error(&self, msg: &str) {
        self.write(&format!("{} {}", style("error:").red().bold(), msg));
    }

    // -- Internal ---------------------------------------------------------

    fn write(&self, msg: &str) {
        let _ = self.term.write_line(msg);
    }

    /// Write a line with a dim HH:MM:SS timestamp prefix.
    fn write_ts(&self, msg: &str) {
        let ts = Local::now().format("%H:%M:%S");
        let _ = self
            .term
            .write_line(&format!("{} {}", style(ts).dim(), msg));
    }

    fn make_spinner(&self, msg: &str) -> ProgressBar {
        if !self.is_tty {
            let pb = ProgressBar::hidden();
            self.write(&format!("  {msg}"));
            return pb;
        }

        let pb = ProgressBar::new_spinner();
        pb.set_style(
            ProgressStyle::default_spinner()
                .tick_strings(&[
                    "\u{2807}", "\u{2819}", "\u{2839}", "\u{2838}", "\u{283c}", "\u{2834}",
                    "\u{2826}", "\u{2827}", "\u{2807}", "\u{280f}",
                ])
                .template("  {spinner:.cyan} {msg}")
                .unwrap(),
        );
        pb.set_message(msg.to_string());
        pb.enable_steady_tick(Duration::from_millis(80));
        pb
    }
}

fn format_duration(ms: u64) -> String {
    if ms < 1000 {
        format!("{}ms", ms)
    } else {
        format!("{:.1}s", ms as f64 / 1000.0)
    }
}
