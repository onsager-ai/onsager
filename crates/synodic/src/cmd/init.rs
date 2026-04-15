use anyhow::Result;
use clap::Args;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::core::pipeline::{self, Stage};

use crate::cmd::orchestrate;
use crate::util;

#[derive(Args)]
pub struct InitCmd {
    /// Project directory (default: current repo root)
    #[arg(long)]
    dir: Option<String>,

    /// Skip Claude Code hooks setup
    #[arg(long)]
    no_claude_hooks: bool,

    /// Skip git hooksPath configuration
    #[arg(long)]
    no_git_hooks: bool,

    /// Skip orchestration pipeline setup
    #[arg(long)]
    no_orchestration: bool,

    /// Language/stack for orchestration: rust, node, python, go (auto-detected if omitted)
    #[arg(long)]
    lang: Option<String>,
}

impl InitCmd {
    pub fn run(self) -> Result<()> {
        let root = match self.dir {
            Some(d) => PathBuf::from(d),
            None => util::find_repo_root()?,
        };

        // --- L2: Claude Code hooks ---
        if !self.no_claude_hooks {
            setup_claude_hooks(&root)?;
        }

        // --- Orchestration: pipeline.yml + GHA workflow ---
        if !self.no_orchestration {
            let lang = match &self.lang {
                Some(name) => orchestrate::parse_lang(name)?,
                None => orchestrate::detect_language(&root),
            };
            // Generate pipeline.yml (reuse existing logic)
            orchestrate::setup_orchestration(&root, &lang, 3)?;

            // Generate simplified GHA workflow (replaces the 340-line bash version)
            write_simplified_workflow(&root)?;
        }

        // --- L1: Git hooks derived from pipeline.yml ---
        if !self.no_git_hooks {
            setup_git_hooks(&root)?;
        }

        Ok(())
    }
}

/// Generate git hooks from pipeline.yml stage fields and configure hooksPath.
///
/// If `.harness/pipeline.yml` exists, generates `.githooks/pre-commit` and
/// `.githooks/pre-push` from checks with `stage: commit` and `stage: push`.
/// Falls back to just setting hooksPath if pipeline.yml doesn't exist.
fn setup_git_hooks(root: &Path) -> Result<()> {
    let githooks_dir = root.join(".githooks");
    std::fs::create_dir_all(&githooks_dir)?;

    // Try to derive hooks from pipeline.yml
    let config_path = root.join(".harness/pipeline.yml");
    if let Ok(config) = pipeline::load_config(&config_path) {
        for (stage, filename) in [(Stage::Commit, "pre-commit"), (Stage::Push, "pre-push")] {
            if let Some(script) = pipeline::generate_hook_script(&config.checks, stage) {
                let hook_path = githooks_dir.join(filename);
                std::fs::write(&hook_path, &script)?;
                #[cfg(unix)]
                set_executable(&hook_path)?;
                eprintln!("L1: generated .githooks/{filename} from pipeline.yml");
            }
        }
    } else {
        eprintln!("L1: no .harness/pipeline.yml, skipping hook generation");
    }

    // Set hooksPath
    let status = Command::new("git")
        .args(["config", "core.hooksPath", ".githooks"])
        .current_dir(root)
        .status()
        .map_err(|e| anyhow::anyhow!("failed to run git config: {e}"))?;

    if status.success() {
        eprintln!("L1: git hooksPath → .githooks/");
    } else {
        eprintln!("Warning: failed to set git core.hooksPath");
    }

    Ok(())
}

/// Write the simplified GHA workflow that delegates to `synodic run`.
fn write_simplified_workflow(root: &Path) -> Result<()> {
    let workflows_dir = root.join(".github/workflows");
    std::fs::create_dir_all(&workflows_dir)?;

    let path = workflows_dir.join("synodic-pipeline.yml");
    // Overwrite the old 340-line version with the simplified one
    let workflow = pipeline::generate_workflow();
    std::fs::write(&path, workflow)?;
    eprintln!("Orchestration: wrote simplified {}", path.display());

    Ok(())
}

/// Create .claude/settings.json and intercept hook for L2 interception.
fn setup_claude_hooks(root: &Path) -> Result<()> {
    let claude_dir = root.join(".claude");
    let hooks_dir = claude_dir.join("hooks");
    std::fs::create_dir_all(&hooks_dir)?;

    // Write intercept.sh (always ensure executable)
    let intercept_path = hooks_dir.join("intercept.sh");
    if !intercept_path.exists() {
        std::fs::write(&intercept_path, INTERCEPT_HOOK)?;
        eprintln!("L2: created {}", intercept_path.display());
    } else {
        eprintln!("L2: {} already exists, skipping", intercept_path.display());
    }
    #[cfg(unix)]
    set_executable(&intercept_path)?;

    // Write settings.json
    let settings_path = claude_dir.join("settings.json");
    if !settings_path.exists() {
        std::fs::write(&settings_path, CLAUDE_SETTINGS)?;
        eprintln!("L2: created {}", settings_path.display());
    } else {
        eprintln!("L2: {} already exists, skipping", settings_path.display());
    }

    Ok(())
}

#[cfg(unix)]
fn set_executable(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = std::fs::metadata(path)?.permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(path, perms)?;
    Ok(())
}

const INTERCEPT_HOOK: &str = r##"#!/usr/bin/env bash
# L2 Interception hook for Claude Code PreToolUse events.
#
# Reads tool call JSON from stdin, evaluates against Synodic's intercept
# rules, and returns the appropriate exit code + output for Claude Code.
#
# Exit 0 = allow, Exit 2 = block (with reason on stderr).
#
# Override flow (interactive only):
#   Block fires -> user prompted -> override with reason -> feedback recorded -> allow

set -euo pipefail

# Fail-open if jq is not available
if ! command -v jq &>/dev/null; then
  cat >/dev/null  # drain stdin
  exit 0
fi

PROJECT_DIR="$(cd "$(dirname "$0")/../.." && pwd)"
SYNODIC_BIN="${SYNODIC_BIN:-${PROJECT_DIR}/rust/target/release/synodic}"

# Fall back to debug build if release doesn't exist
if [[ ! -x "$SYNODIC_BIN" ]]; then
  SYNODIC_BIN="${PROJECT_DIR}/rust/target/debug/synodic"
fi

# Fall back to PATH
if [[ ! -x "$SYNODIC_BIN" ]]; then
  SYNODIC_BIN="$(command -v synodic 2>/dev/null || true)"
fi

# If no binary, allow (don't block the agent on missing build)
if [[ -z "$SYNODIC_BIN" ]] || [[ ! -x "$SYNODIC_BIN" ]]; then
  exit 0
fi

# Read hook input from stdin
INPUT="$(cat)"

# Extract tool_name and tool_input from the hook's JSON payload (fail-open on parse error)
TOOL_NAME="$(echo "$INPUT" | jq -r '.tool_name // empty' 2>/dev/null)" || true
TOOL_INPUT="$(echo "$INPUT" | jq -c '.tool_input // {}' 2>/dev/null)" || TOOL_INPUT='{}'

# If we couldn't parse the input, allow
if [[ -z "$TOOL_NAME" ]]; then
  exit 0
fi

# Call synodic intercept
RESULT="$("$SYNODIC_BIN" intercept --tool "$TOOL_NAME" --input "$TOOL_INPUT" 2>/dev/null)" || {
  # If the command fails, allow (fail-open)
  exit 0
}

DECISION="$(echo "$RESULT" | jq -r '.decision // "allow"' 2>/dev/null)" || true

if [[ "$DECISION" == "block" ]]; then
  REASON="$(echo "$RESULT" | jq -r '.reason // "Blocked by Synodic governance rule"' 2>/dev/null)" || REASON="Blocked by Synodic governance rule"
  RULE="$(echo "$RESULT" | jq -r '.rule // "unknown"' 2>/dev/null)" || RULE="unknown"

  # Interactive override (only when TTY is available)
  if [ -t 2 ]; then
    echo "" >&2
    echo "  Blocked by rule '$RULE': $REASON" >&2
    echo "" >&2

    read -p "  Override? (y/N): " -n 1 -r OVERRIDE </dev/tty 2>/dev/null || OVERRIDE="n"
    echo "" >&2

    if [[ "$OVERRIDE" =~ ^[Yy]$ ]]; then
      read -p "  Reason (optional): " OVERRIDE_REASON </dev/tty 2>/dev/null || OVERRIDE_REASON=""
      echo "" >&2

      # Record override feedback (fail-open)
      "$SYNODIC_BIN" feedback --rule "$RULE" --signal override \
        --tool "$TOOL_NAME" --input "$TOOL_INPUT" \
        ${OVERRIDE_REASON:+--reason "$OVERRIDE_REASON"} 2>/dev/null || true

      echo "  Override recorded. Proceeding." >&2
      exit 0
    else
      # Record confirmed block
      "$SYNODIC_BIN" feedback --rule "$RULE" --signal confirmed \
        --tool "$TOOL_NAME" --input "$TOOL_INPUT" 2>/dev/null || true

      echo "  Action blocked." >&2
      exit 2
    fi
  else
    # Non-interactive — always block, record confirmed
    "$SYNODIC_BIN" feedback --rule "$RULE" --signal confirmed \
      --tool "$TOOL_NAME" --input "$TOOL_INPUT" 2>/dev/null || true

    echo "Synodic L2 interception [$RULE]: $REASON" >&2
    exit 2
  fi
fi

exit 0
"##;

const CLAUDE_SETTINGS: &str = r##"{
  "hooks": {
    "PreToolUse": [
      {
        "matcher": "Bash|Write|Edit",
        "hooks": [
          {
            "type": "command",
            "command": "\"$CLAUDE_PROJECT_DIR\"/.claude/hooks/intercept.sh",
            "timeout": 5,
            "statusMessage": "Synodic L2 intercept check..."
          }
        ]
      }
    ]
  }
}
"##;
