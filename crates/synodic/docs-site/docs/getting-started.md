---
sidebar_position: 2
---

# Getting Started

## Installation

### From source

```bash
git clone https://github.com/codervisor/synodic.git
cd synodic/rust
cargo build --release
# Binary at target/release/synodic
```

## Quick start

### 1. Initialize governance

```bash
cd your-project
synodic init
```

This configures:
- **L1**: `git config core.hooksPath .githooks` (if `.githooks/` exists)
- **L2**: `.claude/settings.json` with `PreToolUse` hook wired to `synodic intercept`
- **L2**: `.claude/hooks/intercept.sh` — adapter script (stdin JSON → CLI → exit code)

### 2. Build the intercept binary

```bash
cd synodic/rust
cargo build --release
```

The hook script looks for the binary at `rust/target/release/synodic` (falls back to `rust/target/debug/synodic`). If neither exists, the hook allows all actions (fail-open).

### 3. Use Claude Code normally

The `PreToolUse` hook runs automatically on every `Bash`, `Write`, and `Edit` tool call. Dangerous actions are blocked with a clear message:

```
Synodic L2 interception [destructive-git]: Block destructive git operations on protected branches
```

Safe actions pass through with no delay.

## Flags

```bash
synodic init --no-git-hooks      # Skip L1 git hooks setup
synodic init --no-claude-hooks   # Skip L2 Claude Code hooks setup
synodic init --dir /path/to/repo # Specify project directory
```
