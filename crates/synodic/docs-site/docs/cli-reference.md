---
sidebar_position: 3
---

# CLI Reference

## Commands

### `synodic init`

Set up L1 git hooks and L2 Claude Code hooks for the current project.

```bash
synodic init [options]
```

| Option | Description |
|--------|-------------|
| `--dir <path>` | Project directory (default: current repo root) |
| `--no-git-hooks` | Skip git hooksPath configuration |
| `--no-claude-hooks` | Skip Claude Code hooks setup |

**L1 setup:** Runs `git config core.hooksPath .githooks` if a `.githooks/` directory exists.

**L2 setup:** Creates `.claude/hooks/intercept.sh` and `.claude/settings.json` wiring `PreToolUse` to the intercept engine.

---

### `synodic intercept`

Evaluate an agent tool call against interception rules. Called by Claude Code's `PreToolUse` hook — not typically invoked directly.

```bash
synodic intercept --tool <name> --input '<json>' [--rules <path>]
```

| Option | Description |
|--------|-------------|
| `--tool` | Tool name (e.g., `Bash`, `Write`, `Edit`) |
| `--input` | Tool input as JSON string |
| `--rules` | Path to custom rules file (JSON). Uses default rules if omitted. |

**Output:** JSON to stdout.

```json
{"decision": "allow"}
```

```json
{"decision": "block", "reason": "Block destructive git operations", "rule": "destructive-git"}
```
