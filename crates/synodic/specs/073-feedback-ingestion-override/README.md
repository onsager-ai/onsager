---
status: complete
created: 2026-03-30
completed: 2026-04-03
priority: critical
tags:
- governance
- feedback
- override
- ux
depends_on:
- "072"
created_at: 2026-03-30T00:00:00Z
updated_at: 2026-03-30T00:00:00Z
---

# Feedback Ingestion & Override UX

> **Status**: complete · **Priority**: critical · **Created**: 2026-03-30

## Overview

The governance feedback loop depends on high-quality signals about rule performance. The most valuable signal is **user override with reasoning** — when a user bypasses a block and explains why. This spec defines:

1. How users override blocks (the UX flow)
2. How override reasons are captured and stored
3. The `synodic feedback` command for recording all signal types
4. Reason clustering for pattern detection
5. API endpoints for feedback ingestion

**Current gap**: The existing `intercept.sh` has no override mechanism. It exits 0 (allow) or 2 (block), but there's no way for the user to say "I disagree with this block." This spec fixes that.

## Design

### Signal types

| Signal | Confidence | Source | Latency | What it tells us |
|--------|-----------|---------|---------|------------------|
| `override` | High | User bypasses block | Immediate | Likely false positive |
| `confirmed` | High | User accepts block | Immediate | Likely true positive |
| `ci_failure` | Medium | CI pipeline | Minutes | Possible false negative (allowed action → broken code) |
| `incident` | High | Incident response | Days–weeks | Definite false negative (harm occurred) |

### Override UX flow

**Current behavior** (broken):
```
Tool call → intercept.sh → synodic intercept → exit 2 → Claude Code aborts tool
User has no recourse
```

**New behavior**:
```
Tool call → intercept.sh → synodic intercept → block decision
  → intercept.sh prompts: "Blocked by rule 'destructive-git'. Override? (y/N): "
  → User types 'y'
  → intercept.sh prompts: "Reason (optional): "
  → User types "test environment"
  → synodic feedback --rule destructive-git --signal override --reason "test environment"
  → intercept.sh exits 0 → tool proceeds
```

**Implementation in intercept.sh**:

```bash
#!/bin/bash
set -euo pipefail

# Read JSON from stdin (Claude Code hook input)
INPUT=$(cat)
TOOL_NAME=$(echo "$INPUT" | jq -r '.tool_name // empty')
TOOL_INPUT=$(echo "$INPUT" | jq -r '.tool_input // empty')

# Evaluate with synodic
RESULT=$(synodic intercept --tool "$TOOL_NAME" --input "$TOOL_INPUT" 2>/dev/null || echo '{"decision":"allow"}')
DECISION=$(echo "$RESULT" | jq -r '.decision // "allow"')

if [ "$DECISION" = "block" ]; then
    RULE=$(echo "$RESULT" | jq -r '.rule // "unknown"')
    REASON=$(echo "$RESULT" | jq -r '.reason // "No reason provided"')

    # Interactive override prompt (only if TTY available)
    if [ -t 0 ]; then
        echo "❌ Blocked by rule '$RULE': $REASON" >&2
        read -p "Override? (y/N): " -n 1 -r OVERRIDE >&2
        echo >&2

        if [[ $OVERRIDE =~ ^[Yy]$ ]]; then
            read -p "Reason (optional): " OVERRIDE_REASON >&2
            # Record override signal
            synodic feedback --rule "$RULE" --signal override ${OVERRIDE_REASON:+--reason "$OVERRIDE_REASON"} >/dev/null 2>&1 || true
            echo "✅ Override recorded. Proceeding..." >&2
            exit 0
        else
            # User declined override — record confirmed block
            synodic feedback --rule "$RULE" --signal confirmed >/dev/null 2>&1 || true
            echo "⛔ Action blocked." >&2
            exit 2
        fi
    else
        # Non-interactive (e.g., CI/automation) — always block
        echo "❌ Blocked by rule '$RULE': $REASON" >&2
        exit 2
    fi
else
    # Allow decision
    exit 0
fi
```

**Key design choices**:

1. **TTY check** (`[ -t 0 ]`): Only prompt if running interactively. In CI/automation, always block (no override).
2. **Fail-open for feedback**: `synodic feedback` errors don't abort — if DB is down, override still proceeds (governance takes priority over telemetry).
3. **Record confirmed blocks**: If user declines override, that's also valuable feedback (rule is correct).
4. **Graceful fallback**: If `jq` or `synodic` is missing, script allows (current fail-open behavior preserved).

### `synodic feedback` command

```
synodic feedback --rule <rule-id> --signal <type> [--reason <text>] [--evidence <url>]
```

**Arguments**:
- `--rule <rule-id>`: Which rule triggered (required)
- `--signal <type>`: override | confirmed | ci_failure | incident (required)
- `--reason <text>`: Free-text explanation (required for override, optional otherwise)
- `--evidence <url>`: Link to CI run, incident report, etc. (optional)
- `--session <id>`: Claude Code session ID (auto-detected from `CLAUDE_SESSION_ID` env var if available)
- `--tool <name>`: Tool that was called (auto-detected from last intercept call if available)
- `--input <json>`: Tool input (auto-detected)

**Implementation**:

```rust
// harness-cli/src/cmd/feedback.rs
pub struct FeedbackCmd {
    rule: String,
    signal: SignalType,
    reason: Option<String>,
    evidence: Option<String>,
    session: Option<String>,
    tool: Option<String>,
    input: Option<String>,
}

impl FeedbackCmd {
    pub async fn run(&self) -> Result<()> {
        let storage = get_storage().await?;

        let event = FeedbackEvent {
            signal_type: self.signal.clone(),
            rule_id: self.rule.clone(),
            session_id: self.session.clone().or_else(|| env::var("CLAUDE_SESSION_ID").ok()),
            tool_name: self.tool.clone().unwrap_or_else(|| "unknown".to_string()),
            tool_input: self.input.clone()
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or(json!({})),
            override_reason: self.reason.clone(),
            failure_type: None,
            evidence_url: self.evidence.clone(),
            project_id: detect_project_id()?,
            created_at: Utc::now(),
        };

        storage.record_feedback(event).await?;

        // Update rule Beta state (α or β increment)
        match self.signal {
            SignalType::Override => {
                // Increment β (false positive evidence)
                storage.update_rule(&self.rule, UpdateRule {
                    beta_increment: Some(1),
                    ..Default::default()
                }).await?;
            }
            SignalType::Confirmed => {
                // Increment α (true positive evidence)
                storage.update_rule(&self.rule, UpdateRule {
                    alpha_increment: Some(1),
                    ..Default::default()
                }).await?;
            }
            _ => {} // CI failure and incident handled by optimize command
        }

        Ok(())
    }
}
```

**Key behaviors**:
- Override signals increment `rules.beta` (false positive count)
- Confirmed signals increment `rules.alpha` (true positive count)
- All signals are recorded in `feedback_events` for audit trail

### Reason clustering

Override reasons are free-text, but patterns emerge. Clustering helps distinguish "rule is wrong" from "rule is right but context-specific."

**Clustering approach**:

```rust
// harness-core/src/clustering.rs
use std::collections::HashMap;

pub struct ReasonCluster {
    pub cluster_id: String,
    pub canonical_reason: String,
    pub reasons: Vec<String>,
    pub count: usize,
}

pub fn cluster_override_reasons(reasons: Vec<String>) -> Vec<ReasonCluster> {
    // Simple keyword-based clustering (can upgrade to embeddings later)
    let mut clusters: HashMap<String, Vec<String>> = HashMap::new();

    for reason in reasons {
        let normalized = reason.to_lowercase();
        let cluster_key = if normalized.contains("test") || normalized.contains("demo") || normalized.contains("throwaway") {
            "non-production"
        } else if normalized.contains("know what") || normalized.contains("intentional") {
            "expert-override"
        } else if normalized.contains("false") || normalized.contains("not dangerous") || normalized.contains("wrong") {
            "rule-error"
        } else {
            "other"
        };

        clusters.entry(cluster_key.to_string())
            .or_insert_with(Vec::new)
            .push(reason);
    }

    clusters.into_iter().map(|(key, reasons)| {
        ReasonCluster {
            cluster_id: key.clone(),
            canonical_reason: match key.as_str() {
                "non-production" => "Action is safe in non-production context".to_string(),
                "expert-override" => "User has domain expertise to judge safety".to_string(),
                "rule-error" => "Rule incorrectly flagged safe action".to_string(),
                _ => "Other context".to_string(),
            },
            count: reasons.len(),
            reasons,
        }
    }).collect()
}
```

**Interpretation**:

| Cluster | Suggested action |
|---------|------------------|
| `non-production` (5+ occurrences) | Add context-aware exception (e.g., allow force-push on non-protected branches) |
| `expert-override` (any frequency) | No rule change — system working as designed (expert judgment) |
| `rule-error` (3+ occurrences) | Flag rule for review — may need narrowing or deprecation |
| `other` | Manual review — unclear pattern |

**Future enhancement**: Upgrade to embedding-based clustering (OpenAI/Voyage embeddings + k-means) for better semantic grouping.

### API endpoints

#### `POST /api/feedback`

Record a feedback event.

**Request**:
```json
{
  "rule_id": "destructive-git",
  "signal_type": "override",
  "tool_name": "Bash",
  "tool_input": {"command": "git push --force"},
  "override_reason": "test environment",
  "session_id": "abc123",
  "project_id": "myapp"
}
```

**Response**:
```json
{
  "id": "550e8400-e29b-41d4-a716-446655440000",
  "created_at": "2026-03-30T12:34:56Z"
}
```

**Implementation**:
```rust
// harness-http/src/api/feedback.rs
use axum::{Json, Extension};
use harness_core::storage::Storage;

pub async fn record(
    Extension(storage): Extension<Arc<dyn Storage>>,
    Json(payload): Json<FeedbackRequest>,
) -> Result<Json<FeedbackResponse>, ApiError> {
    let event = FeedbackEvent {
        signal_type: payload.signal_type,
        rule_id: payload.rule_id.clone(),
        session_id: payload.session_id,
        tool_name: payload.tool_name,
        tool_input: payload.tool_input,
        override_reason: payload.override_reason,
        failure_type: payload.failure_type,
        evidence_url: payload.evidence_url,
        project_id: payload.project_id,
        created_at: Utc::now(),
    };

    storage.record_feedback(event.clone()).await?;

    // Update rule alpha/beta
    match event.signal_type.as_str() {
        "override" => {
            storage.update_rule(&payload.rule_id, UpdateRule {
                beta_increment: Some(1),
                ..Default::default()
            }).await?;
        }
        "confirmed" => {
            storage.update_rule(&payload.rule_id, UpdateRule {
                alpha_increment: Some(1),
                ..Default::default()
            }).await?;
        }
        _ => {}
    }

    Ok(Json(FeedbackResponse {
        id: event.id.unwrap(),
        created_at: event.created_at,
    }))
}
```

#### `GET /api/feedback?rule_id=X&signal_type=override`

Query feedback events with filters.

**Query params**:
- `rule_id` (optional): Filter by rule
- `signal_type` (optional): Filter by signal type
- `session_id` (optional): Filter by session
- `since` (optional): ISO 8601 timestamp (e.g., `2026-03-01T00:00:00Z`)
- `limit` (optional): Max results (default: 100)

**Response**:
```json
{
  "events": [
    {
      "id": "...",
      "rule_id": "destructive-git",
      "signal_type": "override",
      "override_reason": "test environment",
      "created_at": "2026-03-30T12:34:56Z"
    }
  ],
  "total": 42
}
```

#### `GET /api/overrides/:rule_id/clusters`

Get clustered override reasons for a specific rule.

**Response**:
```json
{
  "rule_id": "destructive-git",
  "clusters": [
    {
      "cluster_id": "non-production",
      "canonical_reason": "Action is safe in non-production context",
      "count": 12,
      "example_reasons": [
        "test environment",
        "throwaway branch",
        "CI branch"
      ]
    }
  ]
}
```

### Dashboard integration

Update `packages/ui/src/api.ts` to include feedback endpoints:

```typescript
export interface FeedbackEvent {
  id: string
  rule_id: string
  signal_type: 'override' | 'confirmed' | 'ci_failure' | 'incident'
  override_reason?: string
  created_at: string
}

export interface ReasonCluster {
  cluster_id: string
  canonical_reason: string
  count: number
  example_reasons: string[]
}

export async function recordFeedback(event: Omit<FeedbackEvent, 'id' | 'created_at'>): Promise<FeedbackEvent> {
  const res = await fetch('/api/feedback', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(event),
  })
  return res.json()
}

export async function getOverrideClusters(ruleId: string): Promise<ReasonCluster[]> {
  const res = await fetch(`/api/overrides/${ruleId}/clusters`)
  const data = await res.json()
  return data.clusters
}
```

**Dashboard view**: Add "Override Reasons" panel showing clustered reasons for each rule, with suggested actions.

## Plan

### Phase 1: Override UX in intercept.sh

- [ ] Update `.claude/hooks/intercept.sh` with interactive override prompt
- [ ] Add TTY detection (interactive vs CI)
- [ ] Test override flow manually with `synodic init` → trigger block → override

### Phase 2: `synodic feedback` command

- [ ] Add `FeedbackCmd` to `harness-cli/src/cmd/feedback.rs`
- [ ] Implement signal recording to DB (via Storage trait)
- [ ] Implement α/β increment on override/confirmed signals
- [ ] CLI output confirms recording: "✅ Override recorded for rule 'destructive-git'"

### Phase 3: Reason clustering

- [ ] Implement keyword-based clustering in `harness-core/src/clustering.rs`
- [ ] CLI command `synodic feedback analyze --rule <id>` shows clusters
- [ ] Store cluster results in DB (optional: `override_clusters` table)

### Phase 4: API endpoints

- [ ] `POST /api/feedback` endpoint in `harness-http`
- [ ] `GET /api/feedback` with filters
- [ ] `GET /api/overrides/:rule_id/clusters`
- [ ] Integration tests for all endpoints

### Phase 5: Dashboard integration

- [ ] Update `ui/src/api.ts` with feedback types and methods
- [ ] Add "Override Reasons" panel to rule detail view
- [ ] Display cluster suggestions (e.g., "12 users said 'test environment' — consider adding context-aware exception")

## Test

### Unit tests
- [ ] `synodic feedback --rule X --signal override` increments rule β by 1
- [ ] `synodic feedback --rule X --signal confirmed` increments rule α by 1
- [ ] Feedback event is recorded in `feedback_events` table with correct signal type
- [ ] Override reason is stored in `override_reason` field
- [ ] CI failure signal does not increment α or β (handled by optimize command)

### Integration tests
- [ ] Override flow in intercept.sh: block → user types 'y' → reason prompt → feedback recorded → exit 0
- [ ] Confirmed block flow: block → user types 'n' → feedback recorded → exit 2
- [ ] Non-interactive mode: no TTY → always exit 2 (no override prompt)
- [ ] `POST /api/feedback` returns 201 and increments α/β correctly
- [ ] `GET /api/feedback?rule_id=X&signal_type=override` returns only override events for rule X

### Clustering tests
- [ ] Reasons ["test env", "demo mode", "throwaway"] cluster as "non-production"
- [ ] Reasons ["I know what I'm doing", "intentional"] cluster as "expert-override"
- [ ] Reasons ["false alarm", "not dangerous"] cluster as "rule-error"
- [ ] `GET /api/overrides/:rule_id/clusters` returns clusters with counts

### End-to-end
- [ ] Trigger block → override with reason "test env" → reason appears in dashboard → clustered as "non-production"
- [ ] 5 overrides with "test env" → dashboard suggests adding context-aware exception

## Notes

### Why not auto-override on high β/(α+β)?

Some governance systems auto-disable rules when false positive rate exceeds a threshold. We don't because:

1. **Context matters**: 10 overrides with reason "test environment" suggests adding an exception, not disabling the rule entirely.
2. **Expert judgment**: "I know what I'm doing" overrides are not evidence the rule is wrong — they're evidence the user has domain knowledge to make exceptions.
3. **Human-in-the-loop**: Rule deprecation is a high-impact decision (spec 072 evidentiary standards). It requires human review of clustered reasons, not just a β count.

Auto-flagging for review (β/(α+β) > 0.4) is fine. Auto-deprecation is not.

### Override abuse prevention

Could users just override every block, rendering governance useless? Mitigations:

1. **Friction**: Typing a reason adds friction — users won't do it unless they genuinely disagree.
2. **Audit trail**: Every override is logged with session ID, user, timestamp. Suspicious patterns (user X overrides 50 times in a day) are visible.
3. **Constitutional constraint**: The "right-of-appeal" constraint (spec 071) explicitly allows overrides — this is by design, not a bug. Governance without escape hatches becomes adversarial.

If abuse is observed, the correct response is not to remove overrides but to investigate why users disagree with the rules so frequently.

### Future: LLM-based reason clustering

The keyword-based clustering is a V1. Future enhancement:

1. Embed all override reasons using an embedding model (OpenAI, Voyage, sentence-transformers)
2. Cluster embeddings with k-means or HDBSCAN
3. Use LLM to generate cluster summaries (e.g., GPT-4 given 10 reasons → "Users are force-pushing to feature branches for rebasing workflows")
4. Suggest rule refinements via LLM (e.g., "Allow force-push on branches matching `feature/*`")

This turns qualitative feedback into quantitative rule improvements at scale.
