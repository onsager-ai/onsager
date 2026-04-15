---
sidebar_position: 1
slug: /
---

# Synodic

**The tool that watches the AI agents.**

Synodic is open-source AI agent governance via hooks — enforcing rules on AI coding agent sessions through standard git and Claude Code hook mechanisms.

## What it does

- **L1: Git hooks** — deterministic checks (format, lint, test) on commit and push
- **L2: Claude Code hooks** — real-time pattern-based blocking of dangerous tool calls
- **No databases, no log files** — governance is enforced through standard hook mechanisms

## Default interception rules

| Rule | Blocks | Tools |
|------|--------|-------|
| `destructive-git` | `git reset --hard`, `git push --force`, `git clean -fd` | Bash |
| `secrets-in-args` | API keys, passwords, tokens in tool arguments | All |
| `writes-outside-project` | Writes to `/etc/**` | Write, Edit |
| `writes-to-system` | Writes to `/usr/**` | Write, Edit |
| `dangerous-rm` | `rm -rf /`, `rm -rf ~` | Bash |

## Architecture

Synodic is a minimal Rust workspace with two crates:

```
rust/
├── harness-core    # L2 interception engine (pattern matching, rules)
└── harness-cli     # CLI: init + intercept
```

### Related repositories

- **[codervisor/eval](https://github.com/codervisor/eval)** — Standalone eval framework
- **[codervisor/orchestra](https://github.com/codervisor/orchestra)** — Coordination patterns (pipeline, fractal, swarm)
