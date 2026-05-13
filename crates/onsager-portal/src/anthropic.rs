//! Minimal Anthropic Messages API client for portal's server-side AI
//! features. Today's only caller is `propose_remediation` (#312) —
//! kept narrow to that surface intentionally; if a second caller
//! appears, generalize then, not now.
//!
//! Why a hand-rolled client (vs. an SDK):
//!
//! - reqwest is already a portal dep; pulling in `anthropic-sdk` for
//!   one call would be a fresh trust surface plus extra build cost.
//! - We need fine-grained control over `cache_control` blocks (prompt
//!   caching) and `usage.cache_creation_input_tokens` /
//!   `cache_read_input_tokens` (cost ledger). The shape is small
//!   enough that the SDK's abstractions would obscure rather than help.
//!
//! Mirrors the pattern in `synodic/src/core/llm.rs` (also direct
//! reqwest), kept independent because synodic is a sibling subsystem
//! that the seam rule forbids portal from importing.
//!
//! ## Prompt caching
//!
//! Anthropic prompt caching keys on the exact bytes of the cached
//! prefix. To keep the cache warm across `propose_remediation` calls
//! within a workspace, the **system** block carries the workspace-
//! invariant context (tool registry summary, response-format
//! instructions, naming conventions) and is marked
//! `{"type":"text", "text":"...", "cache_control":{"type":"ephemeral"}}`.
//! The per-call failure summary rides in the **user** message and is
//! not cached.

use std::time::Duration;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

const ANTHROPIC_API_BASE: &str = "https://api.anthropic.com/v1";
const ANTHROPIC_VERSION: &str = "2023-06-01";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);

/// Default Claude model — the "Sonnet for cost" branch from spec #312.
/// Callers can override via `ProposeRemediationArgs::model`.
pub const DEFAULT_MODEL: &str = "claude-sonnet-4-6";

/// Higher-reasoning model — the "Opus for hard cases" branch.
pub const OPUS_MODEL: &str = "claude-opus-4-7";

/// Maximum tokens we'll request from the model. Remediation responses
/// are tool-call JSON plus a short rationale per action; 2k is
/// comfortably above what real responses need without budget surprises.
pub const MAX_OUTPUT_TOKENS: u32 = 2048;

/// Resolve a caller-supplied model string to a canonical model id.
/// `"sonnet"` and `"opus"` are convenience aliases; anything else is
/// passed through so callers can pin to a specific dated revision.
pub fn resolve_model(requested: Option<&str>) -> &str {
    match requested.map(str::trim).filter(|s| !s.is_empty()) {
        Some("sonnet") => DEFAULT_MODEL,
        Some("opus") => OPUS_MODEL,
        Some(other) => match other {
            // Reject anything that doesn't look like a Claude model
            // string. Caller falls through to default; this guards
            // against the model-name being a vehicle for prompt
            // injection-via-config.
            s if s.starts_with("claude-") => s,
            _ => DEFAULT_MODEL,
        },
        None => DEFAULT_MODEL,
    }
}

// ── Request / response shapes ───────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct MessagesRequest<'a> {
    pub model: &'a str,
    pub max_tokens: u32,
    pub system: Vec<SystemBlock<'a>>,
    pub messages: Vec<UserMessage<'a>>,
}

#[derive(Debug, Serialize)]
pub struct SystemBlock<'a> {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub text: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_control: Option<CacheControl>,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct CacheControl {
    #[serde(rename = "type")]
    pub kind: &'static str,
}

impl CacheControl {
    pub const EPHEMERAL: Self = CacheControl { kind: "ephemeral" };
}

#[derive(Debug, Serialize)]
pub struct UserMessage<'a> {
    pub role: &'static str,
    pub content: &'a str,
}

#[derive(Debug, Deserialize)]
pub struct MessagesResponse {
    #[serde(default)]
    pub content: Vec<ContentBlock>,
    #[serde(default)]
    pub usage: Usage,
    #[serde(default)]
    pub model: String,
}

#[derive(Debug, Deserialize)]
pub struct ContentBlock {
    #[serde(rename = "type", default)]
    pub kind: String,
    #[serde(default)]
    pub text: Option<String>,
}

#[derive(Debug, Default, Deserialize, Clone)]
pub struct Usage {
    #[serde(default)]
    pub input_tokens: u32,
    #[serde(default)]
    pub output_tokens: u32,
    #[serde(default)]
    pub cache_creation_input_tokens: u32,
    #[serde(default)]
    pub cache_read_input_tokens: u32,
}

/// Concatenate all text content blocks. Anthropic returns content as a
/// list of typed blocks (text, tool_use, ...); for our request shape
/// we expect text-only.
pub fn collect_text(content: &[ContentBlock]) -> String {
    content
        .iter()
        .filter(|c| c.kind == "text")
        .filter_map(|c| c.text.clone())
        .collect::<Vec<_>>()
        .join("")
}

// ── Pricing ────────────────────────────────────────────────────────

/// Per-million-token pricing snapshot (USD). Updated alongside model
/// releases. Sonnet 4.6 / Opus 4.7 prices are the published list as of
/// 2026-05. Cache writes are billed at ~1.25x input; cache reads at
/// ~0.1x input. We charge per Anthropic's documented rates rather than
/// the wire data because the wire usage object doesn't carry the price.
#[derive(Debug, Clone, Copy)]
pub struct ModelPricing {
    pub input_per_mtok_usd: f64,
    pub output_per_mtok_usd: f64,
    pub cache_write_per_mtok_usd: f64,
    pub cache_read_per_mtok_usd: f64,
}

impl ModelPricing {
    pub const SONNET_4_6: Self = Self {
        input_per_mtok_usd: 3.0,
        output_per_mtok_usd: 15.0,
        cache_write_per_mtok_usd: 3.75,
        cache_read_per_mtok_usd: 0.30,
    };
    pub const OPUS_4_7: Self = Self {
        input_per_mtok_usd: 15.0,
        output_per_mtok_usd: 75.0,
        cache_write_per_mtok_usd: 18.75,
        cache_read_per_mtok_usd: 1.50,
    };

    pub fn for_model(model: &str) -> Self {
        if model.contains("opus") {
            Self::OPUS_4_7
        } else {
            // Default to Sonnet pricing for anything we don't recognize.
            // Worst case the cost is slightly under-reported for a more
            // expensive model; the call still completes and the workspace
            // still pays the upstream bill.
            Self::SONNET_4_6
        }
    }

    pub fn estimate_usd(&self, usage: &Usage) -> f64 {
        let m = 1_000_000.0;
        (usage.input_tokens as f64 * self.input_per_mtok_usd
            + usage.output_tokens as f64 * self.output_per_mtok_usd
            + usage.cache_creation_input_tokens as f64 * self.cache_write_per_mtok_usd
            + usage.cache_read_input_tokens as f64 * self.cache_read_per_mtok_usd)
            / m
    }
}

// ── Client ─────────────────────────────────────────────────────────

pub struct AnthropicClient {
    api_key: String,
    base_url: String,
    http: reqwest::Client,
}

impl AnthropicClient {
    pub fn new(api_key: String) -> Result<Self> {
        let base_url =
            std::env::var("ANTHROPIC_API_BASE").unwrap_or_else(|_| ANTHROPIC_API_BASE.to_string());
        let http = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .build()
            .context("build anthropic http client")?;
        Ok(Self {
            api_key,
            base_url,
            http,
        })
    }

    /// Forward a fully-formed Anthropic Messages API request body verbatim,
    /// injecting only the auth header and the prompt-caching beta. Used by
    /// the `/api/chat/completions` relay (spec #318) so the dashboard never
    /// holds an API key.
    pub async fn forward(&self, body: &serde_json::Value) -> Result<serde_json::Value> {
        let url = format!("{}/messages", self.base_url);
        let resp = self
            .http
            .post(&url)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("anthropic-beta", "prompt-caching-2024-07-31")
            .header("content-type", "application/json")
            .json(body)
            .send()
            .await
            .context("anthropic relay request failed")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            tracing::warn!(
                status = %status,
                body = %body,
                "anthropic relay: non-2xx"
            );
            anyhow::bail!("anthropic API returned {status}");
        }

        resp.json::<serde_json::Value>()
            .await
            .context("decode anthropic relay response")
    }

    pub async fn messages(&self, req: &MessagesRequest<'_>) -> Result<MessagesResponse> {
        let url = format!("{}/messages", self.base_url);
        let resp = self
            .http
            .post(&url)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("content-type", "application/json")
            .json(req)
            .send()
            .await
            .context("anthropic request failed")?;

        if !resp.status().is_success() {
            let status = resp.status();
            // Pull the body for server-side diagnostics only — callers
            // surface this through `stub_reason`, so we keep the
            // bubbled-up error short and stable instead of leaking
            // raw provider output.
            let body = resp.text().await.unwrap_or_default();
            tracing::warn!(
                status = %status,
                body = %body,
                "anthropic API returned non-2xx"
            );
            anyhow::bail!("anthropic API returned {status}");
        }

        let parsed: MessagesResponse = resp.json().await.context("decode anthropic response")?;
        Ok(parsed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_model_aliases() {
        assert_eq!(resolve_model(Some("sonnet")), DEFAULT_MODEL);
        assert_eq!(resolve_model(Some("opus")), OPUS_MODEL);
        assert_eq!(resolve_model(None), DEFAULT_MODEL);
        assert_eq!(resolve_model(Some("")), DEFAULT_MODEL);
        assert_eq!(resolve_model(Some("  ")), DEFAULT_MODEL);
    }

    #[test]
    fn resolve_model_passes_through_canonical_ids() {
        assert_eq!(resolve_model(Some("claude-opus-4-7")), "claude-opus-4-7");
        assert_eq!(
            resolve_model(Some("claude-sonnet-4-20250514")),
            "claude-sonnet-4-20250514"
        );
    }

    #[test]
    fn resolve_model_rejects_non_claude_strings() {
        // Guards against `model` being weaponized to point at an
        // attacker-controlled / unrelated model id.
        assert_eq!(resolve_model(Some("gpt-4o")), DEFAULT_MODEL);
        assert_eq!(resolve_model(Some("../etc/passwd")), DEFAULT_MODEL);
    }

    #[test]
    fn estimate_usd_handles_all_token_kinds() {
        let usage = Usage {
            input_tokens: 1_000_000,
            output_tokens: 0,
            cache_creation_input_tokens: 0,
            cache_read_input_tokens: 0,
        };
        let cost = ModelPricing::SONNET_4_6.estimate_usd(&usage);
        assert!((cost - 3.0).abs() < 1e-9);

        let usage = Usage {
            input_tokens: 0,
            output_tokens: 1_000_000,
            cache_creation_input_tokens: 0,
            cache_read_input_tokens: 0,
        };
        let cost = ModelPricing::SONNET_4_6.estimate_usd(&usage);
        assert!((cost - 15.0).abs() < 1e-9);

        let usage = Usage {
            input_tokens: 0,
            output_tokens: 0,
            cache_creation_input_tokens: 1_000_000,
            cache_read_input_tokens: 1_000_000,
        };
        let cost = ModelPricing::SONNET_4_6.estimate_usd(&usage);
        // 3.75 + 0.30
        assert!((cost - 4.05).abs() < 1e-9);
    }

    #[test]
    fn pricing_picks_opus_table_for_opus_models() {
        let p = ModelPricing::for_model("claude-opus-4-7");
        assert_eq!(p.input_per_mtok_usd, 15.0);
        let p = ModelPricing::for_model("claude-sonnet-4-6");
        assert_eq!(p.input_per_mtok_usd, 3.0);
    }

    #[test]
    fn collect_text_concatenates_text_blocks_only() {
        let blocks = vec![
            ContentBlock {
                kind: "text".into(),
                text: Some("hello ".into()),
            },
            ContentBlock {
                kind: "tool_use".into(),
                text: Some("ignored".into()),
            },
            ContentBlock {
                kind: "text".into(),
                text: Some("world".into()),
            },
        ];
        assert_eq!(collect_text(&blocks), "hello world");
    }
}
