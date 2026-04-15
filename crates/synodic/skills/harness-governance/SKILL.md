# harness-governance

AI agent self-reporting skill for Synodic governance.

## Event types
- `tool_call_error` — tool execution failures
- `hallucination` — references to nonexistent files/APIs
- `compliance_violation` — secrets, dangerous commands, prod access
- `misalignment` — agent actions diverge from user intent

## Self-audit checklist
At the end of major tasks, review:
1. Did any tool calls fail? Report as `tool_call_error`
2. Did you reference files that don't exist? Report as `hallucination`
3. Did you access secrets or run dangerous commands? Report as `compliance_violation`
4. Did the result match the user's intent? If not, report as `misalignment`

## Usage
```bash
synodic submit --type <type> --title "<title>" --severity <low|medium|high|critical>
synodic collect --source claude --since 1h
```
