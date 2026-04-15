---
sidebar_position: 5
---

# Agent Integration

Synodic integrates with Claude Code via the standard hooks system.

## How it works

The `PreToolUse` hook intercepts tool calls before execution:

1. Claude Code sends JSON on stdin (tool name, tool input, session metadata)
2. `intercept.sh` extracts the tool name and input
3. `synodic intercept` evaluates against interception rules
4. Exit 0 = allow, Exit 2 = block (with reason on stderr)

## Setup

```bash
synodic init
```

This creates `.claude/settings.json` with the hook configuration:

```json
{
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
```

## Fail-open design

The hook is designed to never accidentally block the agent:

- Missing `jq` → allow
- Missing `synodic` binary → allow
- Malformed JSON input → allow
- Intercept command failure → allow
- Only an explicit `{"decision": "block"}` response triggers exit 2

## Custom rules

Override the default rules by passing a `--rules` flag. See [Interception Rules](./detection-rules) for the rule format.
