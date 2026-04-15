use anyhow::Result;
use clap::{Args, Subcommand};
use std::path::{Path, PathBuf};

use crate::util;

#[derive(Args)]
pub struct OrchestrationCmd {
    #[command(subcommand)]
    command: OrchestrationSubCmd,
}

#[derive(Subcommand)]
enum OrchestrationSubCmd {
    /// Scaffold orchestration pipeline (GitHub Actions workflow + harness config)
    Init(OrchestrationInitCmd),
}

#[derive(Args)]
struct OrchestrationInitCmd {
    /// Project directory (default: current repo root)
    #[arg(long)]
    dir: Option<String>,

    /// Language/stack: rust, node, python, go (auto-detected if omitted)
    #[arg(long)]
    lang: Option<String>,

    /// Max rework cycles (default: 3)
    #[arg(long, default_value = "3")]
    max_rework: u32,
}

impl OrchestrationCmd {
    pub fn run(self) -> Result<()> {
        match self.command {
            OrchestrationSubCmd::Init(cmd) => cmd.run(),
        }
    }
}

impl OrchestrationInitCmd {
    fn run(self) -> Result<()> {
        let root = match self.dir {
            Some(d) => PathBuf::from(d),
            None => util::find_repo_root()?,
        };

        let lang = match &self.lang {
            Some(name) => parse_lang(name)?,
            None => detect_language(&root),
        };

        setup_orchestration(&root, &lang, self.max_rework)
    }
}

// ── Language detection ─────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum ProjectLang {
    Rust,
    Node { pm: String },
    Python,
    Go,
    Generic,
}

impl std::fmt::Display for ProjectLang {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Rust => write!(f, "rust"),
            Self::Node { pm } => write!(f, "node ({pm})"),
            Self::Python => write!(f, "python"),
            Self::Go => write!(f, "go"),
            Self::Generic => write!(f, "generic"),
        }
    }
}

pub fn parse_lang(name: &str) -> Result<ProjectLang> {
    match name.to_lowercase().as_str() {
        "rust" | "rs" => Ok(ProjectLang::Rust),
        "node" | "js" | "ts" | "javascript" | "typescript" => {
            Ok(ProjectLang::Node { pm: "npm".into() })
        }
        "python" | "py" => Ok(ProjectLang::Python),
        "go" | "golang" => Ok(ProjectLang::Go),
        other => anyhow::bail!("Unknown language: {other}. Use: rust, node, python, go"),
    }
}

pub fn detect_language(root: &Path) -> ProjectLang {
    if root.join("Cargo.toml").exists() {
        return ProjectLang::Rust;
    }
    if root.join("package.json").exists() {
        let pm = if root.join("pnpm-lock.yaml").exists() {
            "pnpm"
        } else if root.join("yarn.lock").exists() {
            "yarn"
        } else if root.join("bun.lockb").exists() || root.join("bun.lock").exists() {
            "bun"
        } else {
            "npm"
        };
        return ProjectLang::Node { pm: pm.into() };
    }
    if root.join("pyproject.toml").exists()
        || root.join("setup.py").exists()
        || root.join("requirements.txt").exists()
    {
        return ProjectLang::Python;
    }
    if root.join("go.mod").exists() {
        return ProjectLang::Go;
    }
    ProjectLang::Generic
}

// ── Profile: language-specific snippets for templates ──────────────

struct LangProfile {
    language: String,
    /// GitHub Actions setup steps (YAML, indented 6 spaces)
    gha_setup: String,
    /// Bash snippet for INSPECT phase checks
    inspect_checks: String,
    /// Instruction for the agent to fix formatting before commit
    format_fix: String,
    /// .harness/pipeline.yml checks section
    pipeline_checks: String,
}

fn profile_for(lang: &ProjectLang) -> LangProfile {
    match lang {
        ProjectLang::Rust => LangProfile {
            language: "rust".into(),
            gha_setup: r#"      - name: Install Rust toolchain
        uses: dtolnay/rust-toolchain@stable
        with:
          components: clippy, rustfmt

      - name: Cache cargo
        uses: actions/cache@v4
        with:
          path: |
            ~/.cargo/registry
            ~/.cargo/git
            target
          key: ${{ runner.os }}-cargo-${{ hashFiles('Cargo.lock') }}"#
                .into(),
            inspect_checks: r#"            echo "  ▸ cargo fmt --check"
            if ! FMT_OUT=$(cargo fmt --all -- --check 2>&1); then
              QA_PASSED=false
              FEEDBACK="${FEEDBACK}
            ### Formatting (cargo fmt)
            \`\`\`
            ${FMT_OUT}
            \`\`\`
            Run \`cargo fmt --all\` to fix."
              echo "    FAIL"
            else
              echo "    PASS"
            fi

            echo "  ▸ cargo clippy"
            if ! CLIPPY_OUT=$(cargo clippy --all-targets -- -D warnings 2>&1); then
              QA_PASSED=false
              FEEDBACK="${FEEDBACK}
            ### Lint (cargo clippy)
            \`\`\`
            $(echo "$CLIPPY_OUT" | tail -40)
            \`\`\`"
              echo "    FAIL"
            else
              echo "    PASS"
            fi

            echo "  ▸ cargo test"
            if ! TEST_OUT=$(cargo test 2>&1); then
              QA_PASSED=false
              FEEDBACK="${FEEDBACK}
            ### Tests (cargo test)
            \`\`\`
            $(echo "$TEST_OUT" | tail -40)
            \`\`\`"
              echo "    FAIL"
            else
              echo "    PASS"
            fi"#
            .into(),
            format_fix: r"Run `cargo fmt --all` before finishing".into(),
            pipeline_checks: r#"checks:
  - name: format
    run: "cargo fmt --all -- --check"
    fix: "cargo fmt --all"
  - name: lint
    run: "cargo clippy --all-targets -- -D warnings"
  - name: test
    run: "cargo test""#
                .into(),
        },
        ProjectLang::Node { pm } => {
            let install = match pm.as_str() {
                "pnpm" => "pnpm install --frozen-lockfile",
                "yarn" => "yarn install --frozen-lockfile",
                "bun" => "bun install --frozen-lockfile",
                _ => "npm ci",
            };
            let run_cmd = match pm.as_str() {
                "pnpm" => "pnpm run",
                "yarn" => "yarn run",
                "bun" => "bun run",
                _ => "npm run",
            };
            let test_cmd = match pm.as_str() {
                "pnpm" => "pnpm test",
                "yarn" => "yarn test",
                "bun" => "bun test",
                _ => "npm test",
            };
            let setup_action = match pm.as_str() {
                "pnpm" => "\n      - uses: pnpm/action-setup@v4\n",
                "bun" => "\n      - uses: oven-sh/setup-bun@v2\n",
                _ => "",
            };
            LangProfile {
                language: "node".into(),
                gha_setup: format!(
                    r#"      - uses: actions/setup-node@v4
        with:
          node-version: "22"
{setup_action}
      - name: Install dependencies
        run: {install}"#
                ),
                inspect_checks: format!(
                    r#"            echo "  ▸ {run_cmd} lint"
            if ! LINT_OUT=$({run_cmd} lint 2>&1); then
              QA_PASSED=false
              FEEDBACK="${{FEEDBACK}}
            ### Lint
            \`\`\`
            $(echo "$LINT_OUT" | tail -40)
            \`\`\`"
              echo "    FAIL"
            else
              echo "    PASS"
            fi

            echo "  ▸ {test_cmd}"
            if ! TEST_OUT=$({test_cmd} 2>&1); then
              QA_PASSED=false
              FEEDBACK="${{FEEDBACK}}
            ### Tests
            \`\`\`
            $(echo "$TEST_OUT" | tail -40)
            \`\`\`"
              echo "    FAIL"
            else
              echo "    PASS"
            fi

            echo "  ▸ {run_cmd} build"
            if ! BUILD_OUT=$({run_cmd} build 2>&1); then
              QA_PASSED=false
              FEEDBACK="${{FEEDBACK}}
            ### Build
            \`\`\`
            $(echo "$BUILD_OUT" | tail -20)
            \`\`\`"
              echo "    FAIL"
            else
              echo "    PASS"
            fi"#
                ),
                format_fix: format!("Run `{run_cmd} lint -- --fix` before finishing"),
                pipeline_checks: format!(
                    r#"checks:
  - name: lint
    run: "{run_cmd} lint"
    fix: "{run_cmd} lint -- --fix"
  - name: test
    run: "{test_cmd}"
  - name: build
    run: "{run_cmd} build""#
                ),
            }
        }
        ProjectLang::Python => LangProfile {
            language: "python".into(),
            gha_setup: r#"      - uses: actions/setup-python@v5
        with:
          python-version: "3.12"

      - name: Install dependencies
        run: |
          pip install -e ".[dev]" 2>/dev/null || pip install -r requirements.txt 2>/dev/null || true
          pip install ruff pytest 2>/dev/null || true"#
                .into(),
            inspect_checks: r#"            echo "  ▸ ruff check"
            if ! LINT_OUT=$(ruff check . 2>&1); then
              QA_PASSED=false
              FEEDBACK="${FEEDBACK}
            ### Lint (ruff)
            \`\`\`
            $(echo "$LINT_OUT" | tail -40)
            \`\`\`
            Run \`ruff check --fix .\` to fix."
              echo "    FAIL"
            else
              echo "    PASS"
            fi

            echo "  ▸ ruff format --check"
            if ! FMT_OUT=$(ruff format --check . 2>&1); then
              QA_PASSED=false
              FEEDBACK="${FEEDBACK}
            ### Formatting (ruff format)
            \`\`\`
            ${FMT_OUT}
            \`\`\`
            Run \`ruff format .\` to fix."
              echo "    FAIL"
            else
              echo "    PASS"
            fi

            echo "  ▸ pytest"
            if ! TEST_OUT=$(pytest 2>&1); then
              QA_PASSED=false
              FEEDBACK="${FEEDBACK}
            ### Tests (pytest)
            \`\`\`
            $(echo "$TEST_OUT" | tail -40)
            \`\`\`"
              echo "    FAIL"
            else
              echo "    PASS"
            fi"#
            .into(),
            format_fix: r"Run `ruff format . && ruff check --fix .` before finishing".into(),
            pipeline_checks: r#"checks:
  - name: lint
    run: "ruff check ."
    fix: "ruff check --fix ."
  - name: format
    run: "ruff format --check ."
    fix: "ruff format ."
  - name: test
    run: "pytest""#
                .into(),
        },
        ProjectLang::Go => LangProfile {
            language: "go".into(),
            gha_setup: r#"      - uses: actions/setup-go@v5
        with:
          go-version: "stable"

      - name: Download modules
        run: go mod download"#
                .into(),
            inspect_checks: r#"            echo "  ▸ go vet"
            if ! VET_OUT=$(go vet ./... 2>&1); then
              QA_PASSED=false
              FEEDBACK="${FEEDBACK}
            ### Vet (go vet)
            \`\`\`
            ${VET_OUT}
            \`\`\`"
              echo "    FAIL"
            else
              echo "    PASS"
            fi

            echo "  ▸ go test"
            if ! TEST_OUT=$(go test ./... 2>&1); then
              QA_PASSED=false
              FEEDBACK="${FEEDBACK}
            ### Tests (go test)
            \`\`\`
            $(echo "$TEST_OUT" | tail -40)
            \`\`\`"
              echo "    FAIL"
            else
              echo "    PASS"
            fi"#
            .into(),
            format_fix: r"Run `gofmt -w .` before finishing".into(),
            pipeline_checks: r#"checks:
  - name: vet
    run: "go vet ./..."
  - name: test
    run: "go test ./...""#
                .into(),
        },
        ProjectLang::Generic => LangProfile {
            language: "generic".into(),
            gha_setup: r#"      # TODO: Add setup steps for your project
      - name: Setup
        run: echo "Add your toolchain setup here""#
                .into(),
            inspect_checks: r#"            # TODO: Add your project's quality checks
            echo "  ▸ custom checks"
            if [ -x .harness/scripts/static_gate.sh ]; then
              if ! GATE_OUT=$(.harness/scripts/static_gate.sh 2>&1); then
                QA_PASSED=false
                FEEDBACK="${FEEDBACK}
            ### Custom checks
            \`\`\`
            ${GATE_OUT}
            \`\`\`"
                echo "    FAIL"
              else
                echo "    PASS"
              fi
            else
              echo "    SKIP (no .harness/scripts/static_gate.sh)"
            fi"#
            .into(),
            format_fix: r"Run your project's formatter before finishing".into(),
            pipeline_checks: r#"# TODO: Define your project's quality checks
checks:
  - name: custom
    run: ".harness/scripts/static_gate.sh""#
                .into(),
        },
    }
}

// ── Orchestration setup (called from init and orchestrate init) ────

pub fn setup_orchestration(root: &Path, lang: &ProjectLang, max_rework: u32) -> Result<()> {
    let profile = profile_for(lang);

    write_workflow(root, &profile, max_rework)?;
    write_pipeline_config(root, &profile, max_rework)?;
    write_static_gate_placeholder(root)?;

    eprintln!();
    eprintln!("Orchestration: scaffolded for {lang}");
    eprintln!();
    eprintln!("  Next steps:");
    eprintln!("  1. Add ANTHROPIC_API_KEY to your repo's GitHub Actions secrets");
    eprintln!("  2. Review .harness/pipeline.yml and .github/workflows/synodic-pipeline.yml");
    eprintln!("  3. Trigger: Actions → Synodic Pipeline → Run workflow");
    eprintln!("  4. (Optional) Add custom checks in .harness/scripts/static_gate.sh");
    eprintln!();

    Ok(())
}

fn write_workflow(root: &Path, profile: &LangProfile, max_rework: u32) -> Result<()> {
    let workflows_dir = root.join(".github").join("workflows");
    std::fs::create_dir_all(&workflows_dir)?;

    let path = workflows_dir.join("synodic-pipeline.yml");
    if path.exists() {
        eprintln!("Orchestration: {} already exists, skipping", path.display());
        return Ok(());
    }

    let workflow = generate_workflow(profile, max_rework);
    std::fs::write(&path, workflow)?;
    eprintln!("Orchestration: created {}", path.display());

    Ok(())
}

fn write_pipeline_config(root: &Path, profile: &LangProfile, max_rework: u32) -> Result<()> {
    let harness_dir = root.join(".harness");
    std::fs::create_dir_all(&harness_dir)?;

    let path = harness_dir.join("pipeline.yml");
    if path.exists() {
        eprintln!("Orchestration: {} already exists, skipping", path.display());
        return Ok(());
    }

    let semantic_checks = r#"

  # L2 semantic checks (require ANTHROPIC_API_KEY, skip with --no-semantic)
  - name: security-review
    type: semantic
    prompt: "Review the diff for security vulnerabilities: hardcoded secrets, injection vectors, unsafe patterns, credential exposure. Flag anything that could be exploited."
    severity: block

  - name: goal-alignment
    type: semantic
    prompt: "Does the diff match the task? Flag scope creep (changes unrelated to the task), goal drift (implementation doesn't match intent), unrelated TODO/FIXME additions, and hallucinated API references."
    severity: warn"#;

    let config = format!(
        r#"# Synodic pipeline configuration
# Describes the quality gates for the Build→Inspect→PR loop.
# Edit checks to match your project's needs.
#
# Trigger: GitHub Actions → Synodic Pipeline → Run workflow
# Workflow: .github/workflows/synodic-pipeline.yml

language: {lang}

{checks}{semantic}

pipeline:
  max_rework: {max_rework}
  auto_merge: false
"#,
        lang = profile.language,
        checks = profile.pipeline_checks,
        semantic = semantic_checks,
        max_rework = max_rework,
    );

    std::fs::write(&path, config)?;
    eprintln!("Orchestration: created {}", path.display());

    Ok(())
}

fn write_static_gate_placeholder(root: &Path) -> Result<()> {
    let scripts_dir = root.join(".harness").join("scripts");
    std::fs::create_dir_all(&scripts_dir)?;

    let path = scripts_dir.join("static_gate.sh");
    if path.exists() {
        return Ok(());
    }

    std::fs::write(&path, STATIC_GATE_TEMPLATE)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&path)?.permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&path, perms)?;
    }

    eprintln!("Orchestration: created {}", path.display());
    Ok(())
}

const STATIC_GATE_TEMPLATE: &str = r##"#!/usr/bin/env bash
# Custom quality gate for Synodic pipeline INSPECT phase.
#
# This script runs during the INSPECT phase alongside language-specific checks.
# Exit 0 = pass, non-zero = fail (output becomes rework feedback).
#
# Examples:
#   - Check for TODO/FIXME in changed files
#   - Validate API schema compatibility
#   - Run integration tests
#   - Check documentation coverage

set -euo pipefail

# Uncomment and customize:
# echo "Running custom checks..."
# git diff --name-only "${1:-HEAD~1}"..."${2:-HEAD}" | while read -r file; do
#   if grep -n 'TODO\|FIXME\|HACK' "$file" 2>/dev/null; then
#     echo "Warning: unresolved markers in $file"
#   fi
# done

exit 0
"##;

// ── Workflow template generation ──────────────────────────────────

fn generate_workflow(profile: &LangProfile, max_rework: u32) -> String {
    format!(
        r##"# Generated by: synodic orchestrate init
# Docs: https://github.com/codervisor/synodic
#
# Build→Inspect→PR pipeline with governance.
# Trigger manually: Actions → Synodic Pipeline → Run workflow

name: Synodic Pipeline

on:
  workflow_dispatch:
    inputs:
      prompt:
        description: "Task description (what to build, fix, or refactor)"
        required: true
        type: string
      max_rework:
        description: "Max Build↔Inspect rework cycles"
        required: false
        default: "{max_rework}"
        type: string
      auto_merge:
        description: "Auto-merge PR on pass"
        required: false
        default: false
        type: boolean

concurrency:
  group: synodic-pipeline
  cancel-in-progress: false

jobs:
  pipeline:
    name: "Build → Inspect → Merge"
    runs-on: ubuntu-latest
    timeout-minutes: 60
    permissions:
      contents: write
      pull-requests: write
    env:
      BRANCH: synodic/pipeline-${{{{ github.run_number }}}}
      MAX_REWORK: ${{{{ inputs.max_rework }}}}
    steps:
      # ── Setup ──────────────────────────────────────────────────
      - uses: actions/checkout@v4
        with:
          fetch-depth: 0

{gha_setup}

      - name: Install Claude Code
        run: npm install -g @anthropic-ai/claude-code

      - name: Create pipeline branch
        run: |
          git checkout -b "$BRANCH"
          echo "base_sha=$(git rev-parse HEAD)" >> "$GITHUB_ENV"

      # ── Build↔Inspect Loop ────────────────────────────────────
      - name: "Run Build↔Inspect loop"
        env:
          ANTHROPIC_API_KEY: ${{{{ secrets.ANTHROPIC_API_KEY }}}}
          PROMPT: ${{{{ inputs.prompt }}}}
        run: |
          set +e
          mkdir -p .harness/.runs

          ATTEMPT=0
          FEEDBACK=""
          STATUS="running"

          while [ "$ATTEMPT" -lt "$MAX_REWORK" ] && [ "$STATUS" = "running" ]; do
            ATTEMPT=$((ATTEMPT + 1))
            echo ""
            echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
            echo "  Attempt $ATTEMPT / $MAX_REWORK"
            echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

            # ── BUILD: Claude Code agent implements ──
            echo ""
            echo "▸ BUILD"

            if [ -n "$FEEDBACK" ]; then
              FULL_PROMPT="$(cat <<PROMPTEOF
          ## Task
          $PROMPT

          ## Rework Required (attempt $ATTEMPT)
          The previous attempt failed quality checks. Fix ALL issues:

          $FEEDBACK

          Rules:
          - Fix every issue listed above
          - Do not break existing functionality
          - {format_fix}
          - Commit your changes with a clear message
          PROMPTEOF
          )"
            else
              FULL_PROMPT="$(cat <<PROMPTEOF
          ## Task
          $PROMPT

          Rules:
          - Implement the task described above
          - Follow existing code conventions
          - {format_fix}
          - Commit your changes with a clear message
          PROMPTEOF
          )"
            fi

            echo "$FULL_PROMPT" | claude --print - 2>&1 | tee ".harness/.runs/attempt-${{ATTEMPT}}.log" || true

            # ── INSPECT: Quality checks ──
            echo ""
            echo "▸ INSPECT"
            FEEDBACK=""
            QA_PASSED=true

            # Check for changes
            if git diff --quiet "$base_sha"...HEAD 2>/dev/null; then
              echo "  No code changes detected."
              if [ "$ATTEMPT" -eq 1 ]; then
                echo "  Agent produced no changes on first attempt. Exiting."
                STATUS="error"
                break
              fi
            fi

{inspect_checks}

            # Custom gate (if exists)
            if [ -x .harness/scripts/static_gate.sh ]; then
              echo "  ▸ custom checks"
              if ! GATE_OUT=$(.harness/scripts/static_gate.sh "$base_sha" HEAD 2>&1); then
                QA_PASSED=false
                FEEDBACK="${{FEEDBACK}}
            ### Custom checks
            \`\`\`
            ${{GATE_OUT}}
            \`\`\`"
                echo "    FAIL"
              else
                echo "    PASS"
              fi
            fi

            # ── ROUTE ──
            if [ "$QA_PASSED" = true ]; then
              STATUS="passed"
              echo ""
              echo "✓ INSPECT passed on attempt $ATTEMPT"
            else
              echo ""
              echo "✗ INSPECT failed on attempt $ATTEMPT — reworking"
            fi
          done

          echo "status=$STATUS" >> "$GITHUB_ENV"
          echo "attempts=$ATTEMPT" >> "$GITHUB_ENV"

      # ── Create PR ─────────────────────────────────────────────
      - name: Push branch
        if: env.status == 'passed'
        run: git push -u origin "$BRANCH"

      - name: Create Pull Request
        if: env.status == 'passed'
        uses: peter-evans/create-pull-request@v7
        with:
          branch: ${{{{ env.BRANCH }}}}
          title: "synodic: ${{{{ inputs.prompt }}}}"
          body: |
            ## Summary

            Automated pipeline run via [Synodic](https://github.com/codervisor/synodic).

            **Prompt:** ${{{{ inputs.prompt }}}}
            **Attempts:** ${{{{ env.attempts }}}} / ${{{{ inputs.max_rework }}}}
            **Status:** ${{{{ env.status }}}}
            **Triggered by:** @${{{{ github.actor }}}}
            **Run:** ${{{{ github.server_url }}}}/${{{{ github.repository }}}}/actions/runs/${{{{ github.run_id }}}}
          labels: automated,synodic
          delete-branch: true

      # ── Auto-merge (optional) ─────────────────────────────────
      - name: Auto-merge PR
        if: env.status == 'passed' && inputs.auto_merge
        env:
          GH_TOKEN: ${{{{ secrets.GITHUB_TOKEN }}}}
        run: |
          PR_NUM=$(gh pr list --head "$BRANCH" --json number -q '.[0].number')
          if [ -n "$PR_NUM" ]; then
            gh pr merge "$PR_NUM" --squash --delete-branch
            echo "Merged PR #${{PR_NUM}}"
          fi

      # ── Summary ────────────────────────────────────────────────
      - name: Summary
        if: always()
        run: |
          cat >> "$GITHUB_STEP_SUMMARY" <<EOF
          ## Synodic Pipeline

          | | |
          |---|---|
          | **Prompt** | ${{{{ inputs.prompt }}}} |
          | **Status** | \`${{status}}\` |
          | **Attempts** | ${{attempts}} / ${{{{ inputs.max_rework }}}} |
          | **Branch** | \`${{BRANCH}}\` |
          | **Triggered by** | @${{{{ github.actor }}}} |

          \`\`\`
          prompt → [BUILD: Claude Code] → [INSPECT: quality checks] → [ROUTE] → PR
                         ↑                              ↓
                         └────── rework feedback ───────┘  (up to ${{{{ inputs.max_rework }}}}×)
          \`\`\`
          EOF

          if [ "${{status}}" = "passed" ]; then
            echo "Pipeline completed successfully." >> "$GITHUB_STEP_SUMMARY"
          elif [ "${{status}}" = "error" ]; then
            echo "Agent produced no changes." >> "$GITHUB_STEP_SUMMARY"
            exit 1
          else
            echo "Pipeline exhausted ${{{{ inputs.max_rework }}}} rework cycles." >> "$GITHUB_STEP_SUMMARY"
            exit 1
          fi
"##,
        max_rework = max_rework,
        gha_setup = profile.gha_setup,
        format_fix = profile.format_fix,
        inspect_checks = profile.inspect_checks,
    )
}
