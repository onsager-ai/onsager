//! Claude stream-JSON → normalized `agent.*` session events (#632).
//!
//! Ported from arbor's `agent-runner/src/normalize.ts` (shapes verified
//! there against claude 2.1.x): raw stream-json is unversioned and
//! CLI-upgrade-volatile, so all the churn is isolated here. The
//! normalizer is pure — `handle` maps one parsed event to zero or more
//! `(kind, payload)` rows; persistence lives with the caller.
//!
//! Mapping:
//! - `system/init`            → `agent.started` (model)
//! - assistant `text` blocks  → `agent.text` (capped)
//! - assistant `tool_use`     → `agent.tool_use` (tool, id, input excerpt)
//! - user `tool_result`       → `agent.tool_result` (paired by id; orphans
//!   keep tool "unknown", never dropped)
//! - `result`                 → `agent.completed` (turns / cost / duration)
//!   + warnings for error subtypes and permission denials
//! - `system/model_fallback`  → `agent.warning`
//! - unknown top-level types  → `agent.warning`, once per distinct type
//! - `rate_limit_event`        → `agent.warning` when degraded or using
//!   overage; healthy `allowed` is silent
//! - `stream_event` text deltas → transient drafts via [`text_delta`]
//!   (never persisted; the SSE live feed forwards them, #634); other
//!   partials and thinking blocks are ignored progress noise
//!
//! Not ported from arbor: the out-of-workspace path watcher (v2 has no
//! per-run workspace root yet) and transient text deltas (polling UI).

use std::collections::{HashMap, HashSet};

use serde_json::{Value, json};

/// Cap for assistant prose per event.
pub const MAX_TEXT_CHARS: usize = 2000;
/// Cap for tool inputs / results / warnings.
pub const MAX_EXCERPT_CHARS: usize = 700;

/// One normalized session event, ready to persist.
#[derive(Debug, Clone, PartialEq)]
pub struct AgentEvent {
    pub kind: &'static str,
    pub payload: Value,
}

fn excerpt(s: &str, cap: usize) -> (String, bool) {
    if s.chars().count() <= cap {
        (s.to_string(), false)
    } else {
        (s.chars().take(cap).collect(), true)
    }
}

fn s<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str)
}

pub struct Normalizer {
    /// tool_use id → tool name, so tool_results can be attributed.
    pending_tools: HashMap<String, String>,
    /// Unknown top-level types warned about — once per type.
    warned_unknown: HashSet<String>,
}

impl Default for Normalizer {
    fn default() -> Self {
        Self::new()
    }
}

impl Normalizer {
    pub fn new() -> Self {
        Self {
            pending_tools: HashMap::new(),
            warned_unknown: HashSet::new(),
        }
    }

    fn warning(message: &str) -> AgentEvent {
        let (text, _) = excerpt(message, MAX_EXCERPT_CHARS);
        AgentEvent {
            kind: "agent.warning",
            payload: json!({ "message": text }),
        }
    }

    /// Normalize one parsed stream-json event into zero or more rows.
    pub fn handle(&mut self, event: &Value) -> Vec<AgentEvent> {
        match s(event, "type") {
            Some("system") => match s(event, "subtype") {
                Some("init") => {
                    let mut payload = json!({});
                    if let Some(model) = s(event, "model") {
                        payload["model"] = json!(model);
                    }
                    vec![AgentEvent {
                        kind: "agent.started",
                        payload,
                    }]
                }
                Some("model_fallback") => vec![Self::warning(&format!("model fallback: {event}"))],
                // Other system subtypes are high-frequency progress noise.
                _ => vec![],
            },
            Some("assistant") => self.handle_assistant(event),
            Some("user") => self.handle_user(event),
            Some("result") => Self::handle_result(event),
            // Partial messages: token deltas flow to the transient
            // delta path (`text_delta`, #634); other stream_event
            // subtypes are progress noise — ignored, not warned.
            Some("stream_event") => vec![],
            Some("rate_limit_event") => {
                // "allowed" arrives on every healthy run — only
                // degradations warrant a warning (arbor parity, #634).
                let info = event.get("rate_limit_info");
                let status = info.and_then(|i| i.get("status")).and_then(Value::as_str);
                if let Some(status) = status
                    && status != "allowed"
                {
                    let kind = info
                        .and_then(|i| i.get("rateLimitType"))
                        .and_then(Value::as_str)
                        .unwrap_or("unknown");
                    return vec![Self::warning(&format!("rate limit {status} ({kind})"))];
                }
                if info.and_then(|i| i.get("isUsingOverage")) == Some(&json!(true)) {
                    return vec![Self::warning("rate limit: consuming overage")];
                }
                vec![]
            }
            Some(other) => {
                if self.warned_unknown.insert(other.to_string()) {
                    vec![Self::warning(&format!(
                        "unknown stream event type: {other}"
                    ))]
                } else {
                    vec![]
                }
            }
            None => vec![],
        }
    }

    fn content_blocks(event: &Value) -> impl Iterator<Item = &Value> {
        event
            .get("message")
            .and_then(|m| m.get("content"))
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
    }

    fn handle_assistant(&mut self, event: &Value) -> Vec<AgentEvent> {
        let mut out = Vec::new();
        for block in Self::content_blocks(event) {
            match s(block, "type") {
                Some("text") => {
                    let text = s(block, "text").unwrap_or("");
                    if text.is_empty() {
                        continue;
                    }
                    let (cut, truncated) = excerpt(text, MAX_TEXT_CHARS);
                    let mut payload = json!({ "text": cut });
                    if truncated {
                        payload["truncated"] = json!(true);
                    }
                    out.push(AgentEvent {
                        kind: "agent.text",
                        payload,
                    });
                }
                Some("tool_use") => {
                    let tool = s(block, "name").unwrap_or("unknown").to_string();
                    let id = s(block, "id").map(str::to_string);
                    if let Some(id) = &id {
                        self.pending_tools.insert(id.clone(), tool.clone());
                    }
                    let input = block.get("input").cloned().unwrap_or(json!({}));
                    let (cut, truncated) = excerpt(&input.to_string(), MAX_EXCERPT_CHARS);
                    let mut payload = json!({ "tool": tool, "input_excerpt": cut });
                    if let Some(id) = id {
                        payload["tool_use_id"] = json!(id);
                    }
                    if truncated {
                        payload["truncated"] = json!(true);
                    }
                    out.push(AgentEvent {
                        kind: "agent.tool_use",
                        payload,
                    });
                }
                // thinking blocks: reserved, ignored.
                _ => {}
            }
        }
        out
    }

    fn handle_user(&mut self, event: &Value) -> Vec<AgentEvent> {
        let mut out = Vec::new();
        for block in Self::content_blocks(event) {
            if s(block, "type") != Some("tool_result") {
                continue;
            }
            let id = s(block, "tool_use_id").map(str::to_string);
            // Orphans (unseen tool_use id) keep "unknown" — never dropped.
            let tool = id
                .as_ref()
                .and_then(|i| self.pending_tools.remove(i))
                .unwrap_or_else(|| "unknown".to_string());
            let is_error = block.get("is_error") == Some(&json!(true));
            let result_text = tool_result_text(block.get("content"));
            let (cut, truncated) = excerpt(&result_text, MAX_EXCERPT_CHARS);
            let mut payload = json!({ "tool": tool });
            if let Some(id) = id {
                payload["tool_use_id"] = json!(id);
            }
            if is_error {
                payload["is_error"] = json!(true);
            }
            if !cut.is_empty() {
                payload["result_excerpt"] = json!(cut);
            }
            if truncated {
                payload["truncated"] = json!(true);
            }
            out.push(AgentEvent {
                kind: "agent.tool_result",
                payload,
            });
        }
        out
    }

    fn handle_result(event: &Value) -> Vec<AgentEvent> {
        let mut out = Vec::new();
        let mut payload = json!({});
        if let Some(turns) = event.get("num_turns").and_then(Value::as_u64) {
            payload["turns"] = json!(turns);
        }
        if let Some(cost) = event.get("total_cost_usd").and_then(Value::as_f64) {
            payload["cost_usd"] = json!(cost);
        }
        if let Some(ms) = event.get("duration_ms").and_then(Value::as_u64) {
            payload["duration_ms"] = json!(ms);
        }
        out.push(AgentEvent {
            kind: "agent.completed",
            payload,
        });

        let denials = event
            .get("permission_denials")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        if !denials.is_empty() {
            let tools: Vec<&str> = denials
                .iter()
                .map(|d| s(d, "tool_name").unwrap_or("unknown"))
                .collect();
            out.push(Self::warning(&format!(
                "{} permission denial(s): {}",
                denials.len(),
                tools.join(", ")
            )));
        }
        let subtype = s(event, "subtype");
        if event.get("is_error") == Some(&json!(true))
            || (subtype.is_some() && subtype != Some("success"))
        {
            let detail = s(event, "result").or(s(event, "error")).unwrap_or("");
            out.push(Self::warning(&format!(
                "claude result subtype \"{}\"{}{}",
                subtype.unwrap_or("unknown"),
                if detail.is_empty() { "" } else { ": " },
                detail
            )));
        }
        out
    }
}

/// Extract the text from a `stream_event` token delta, if this event is
/// one (#634). Transient: forwarded to live SSE subscribers, never
/// persisted — the durable `agent.text` row arrives with the full
/// message.
pub fn text_delta(event: &Value) -> Option<&str> {
    if s(event, "type") != Some("stream_event") {
        return None;
    }
    let inner = event.get("event")?;
    if s(inner, "type") != Some("content_block_delta") {
        return None;
    }
    let delta = inner.get("delta")?;
    if s(delta, "type") != Some("text_delta") {
        return None;
    }
    s(delta, "text").filter(|t| !t.is_empty())
}

/// `tool_result.content` is either a string or an array of typed blocks.
fn tool_result_text(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(blocks)) => blocks
            .iter()
            .filter(|b| s(b, "type") == Some("text"))
            .filter_map(|b| s(b, "text"))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fixture lines in the claude 2.1.x stream-json shapes arbor
    /// verified (2026-06-10) — the normalizer's contract.
    fn lines() -> Vec<Value> {
        vec![
            json!({"type":"system","subtype":"init","session_id":"abc","model":"claude-sonnet-4-6"}),
            json!({"type":"assistant","message":{"content":[
                {"type":"text","text":"I'll write the file now."},
                {"type":"tool_use","id":"tu_1","name":"Write","input":{"file_path":"/tmp/x","content":"hi"}}
            ]}}),
            json!({"type":"user","message":{"content":[
                {"type":"tool_result","tool_use_id":"tu_1","content":"File created"}
            ]}}),
            json!({"type":"stream_event","event":{"type":"content_block_delta"}}),
            json!({"type":"result","subtype":"success","result":"done","num_turns":3,
                   "total_cost_usd":0.0123,"duration_ms":4567}),
        ]
    }

    #[test]
    fn full_session_normalizes_in_order() {
        let mut n = Normalizer::new();
        let events: Vec<AgentEvent> = lines().iter().flat_map(|l| n.handle(l)).collect();
        let kinds: Vec<&str> = events.iter().map(|e| e.kind).collect();
        assert_eq!(
            kinds,
            vec![
                "agent.started",
                "agent.text",
                "agent.tool_use",
                "agent.tool_result",
                "agent.completed",
            ]
        );
        assert_eq!(events[0].payload["model"], "claude-sonnet-4-6");
        assert_eq!(events[2].payload["tool"], "Write");
        assert!(
            events[2].payload["input_excerpt"]
                .as_str()
                .unwrap()
                .contains("file_path")
        );
        // tool_result attributed to its tool_use by id.
        assert_eq!(events[3].payload["tool"], "Write");
        assert_eq!(events[3].payload["result_excerpt"], "File created");
        assert_eq!(events[4].payload["turns"], 3);
        assert_eq!(events[4].payload["cost_usd"], 0.0123);
    }

    #[test]
    fn orphan_tool_result_is_kept_with_unknown_tool() {
        let mut n = Normalizer::new();
        let events = n.handle(&json!({"type":"user","message":{"content":[
            {"type":"tool_result","tool_use_id":"never_seen","content":"out"}
        ]}}));
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, "agent.tool_result");
        assert_eq!(events[0].payload["tool"], "unknown");
    }

    #[test]
    fn error_result_adds_warning() {
        let mut n = Normalizer::new();
        let events = n.handle(
            &json!({"type":"result","subtype":"error_max_turns","is_error":true,"result":"ran out"}),
        );
        let kinds: Vec<&str> = events.iter().map(|e| e.kind).collect();
        assert_eq!(kinds, vec!["agent.completed", "agent.warning"]);
        assert!(
            events[1].payload["message"]
                .as_str()
                .unwrap()
                .contains("error_max_turns")
        );
    }

    #[test]
    fn unknown_types_warn_once_per_type() {
        let mut n = Normalizer::new();
        assert_eq!(n.handle(&json!({"type":"mystery"})).len(), 1);
        assert_eq!(n.handle(&json!({"type":"mystery"})).len(), 0);
        assert_eq!(n.handle(&json!({"type":"other_mystery"})).len(), 1);
    }

    #[test]
    fn long_text_is_capped_and_flagged() {
        let mut n = Normalizer::new();
        let long = "x".repeat(MAX_TEXT_CHARS + 50);
        let events = n.handle(&json!({"type":"assistant","message":{"content":[
            {"type":"text","text": long}
        ]}}));
        assert_eq!(events[0].payload["truncated"], true);
        assert_eq!(
            events[0].payload["text"].as_str().unwrap().chars().count(),
            MAX_TEXT_CHARS
        );
    }

    #[test]
    fn rate_limit_allowed_is_silent_degraded_warns() {
        let mut n = Normalizer::new();
        assert!(
            n.handle(&json!({"type":"rate_limit_event",
                "rate_limit_info":{"status":"allowed"}}))
                .is_empty()
        );
        let warned = n.handle(&json!({"type":"rate_limit_event",
            "rate_limit_info":{"status":"rejected","rateLimitType":"five_hour"}}));
        assert_eq!(warned[0].kind, "agent.warning");
        assert_eq!(
            warned[0].payload["message"],
            "rate limit rejected (five_hour)"
        );
        let overage = n.handle(&json!({"type":"rate_limit_event",
            "rate_limit_info":{"status":"allowed","isUsingOverage":true}}));
        assert_eq!(
            overage[0].payload["message"],
            "rate limit: consuming overage"
        );
        // and it no longer trips the unknown-type warning
        assert!(
            n.handle(&json!({"type":"rate_limit_event",
                "rate_limit_info":{"status":"allowed"}}))
                .is_empty()
        );
    }

    #[test]
    fn text_delta_extracts_only_token_deltas() {
        assert_eq!(
            text_delta(&json!({"type":"stream_event","event":{
                "type":"content_block_delta","delta":{"type":"text_delta","text":"hel"}}})),
            Some("hel")
        );
        assert_eq!(
            text_delta(&json!({"type":"stream_event","event":{
                "type":"content_block_delta","delta":{"type":"input_json_delta","partial_json":"{"}}})),
            None
        );
        assert_eq!(text_delta(&json!({"type":"assistant"})), None);
    }

    #[test]
    fn tool_result_array_content_joins_text_blocks() {
        assert_eq!(
            tool_result_text(Some(&json!([
                {"type":"text","text":"a"},
                {"type":"image"},
                {"type":"text","text":"b"}
            ]))),
            "a\nb"
        );
    }
}
