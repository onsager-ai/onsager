---
sidebar_position: 4
---

# Interception Rules

Synodic uses a two-layer governance model enforced entirely through hooks.

## Layer 1 — Git hooks

L1 rules run as standard git hooks in `.githooks/`. They are deterministic, fast, and tool-agnostic.

| Hook | Checks | When |
|------|--------|------|
| `pre-commit` | `cargo fmt --check` | Every commit |
| `pre-push` | fmt + clippy + test | Every push |

Activated by `synodic init` or `git config core.hooksPath .githooks`.

## Layer 2 — Claude Code hooks

L2 rules block dangerous tool calls in real-time via the `PreToolUse` hook. The intercept engine evaluates tool calls against pattern-based rules.

### Default rules

| Rule ID | Description | Tools | Condition |
|---------|-------------|-------|-----------|
| `destructive-git` | Block `git reset --hard`, `git push --force`, `git clean -fd` | Bash | Command regex |
| `secrets-in-args` | Block API keys, passwords, tokens in tool arguments | All | Pattern regex |
| `writes-outside-project` | Block writes to `/etc/**` | Write, Edit | Path glob |
| `writes-to-system` | Block writes to `/usr/**` | Write, Edit | Path glob |
| `dangerous-rm` | Block `rm -rf /` or `rm -rf ~` | Bash | Command regex |

### Rule conditions

Rules use three condition types:

- **Pattern** — regex match against the full serialized tool input JSON
- **Path** — glob match against `file_path` in Write/Edit tool input
- **Command** — regex match against `command` in Bash tool input

### Custom rules

Pass a custom rules file (JSON) to override defaults:

```bash
synodic intercept --tool Bash --input '{"command":"..."}' --rules my-rules.json
```

Rules file format:

```json
[
  {
    "id": "no-prod-config",
    "description": "Block modifications to production config",
    "tools": ["Write", "Edit"],
    "condition": {
      "type": "path",
      "glob": "**/config/production.*"
    }
  }
]
```

### How it works

```
Agent tool call → Claude Code PreToolUse hook
                      ↓
                intercept.sh (reads stdin JSON)
                      ↓
                synodic intercept --tool X --input '{...}'
                      ↓
                InterceptEngine evaluates against rules
                      ↓
                exit 0 (allow) or exit 2 + stderr (block)
```
