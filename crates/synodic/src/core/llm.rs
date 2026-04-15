//! Lightweight LLM client for L2 semantic checks.
//!
//! Supports both Anthropic and OpenAI-compatible APIs via direct HTTP calls
//! (reqwest). This is intentionally separate from Claude Code sessions — semantic
//! checks ARE the governance layer and must not trigger L2 hooks themselves.
//!
//! Provider selection:
//! - `SYNODIC_LLM_PROVIDER=openai` → OpenAI-compatible API
//! - Otherwise → Anthropic API (default)
//!
//! Environment variables:
//! - Anthropic: `ANTHROPIC_API_KEY`, `ANTHROPIC_API_BASE` (optional override)
//! - OpenAI:    `OPENAI_API_KEY`, `OPENAI_API_BASE` (defaults to api.openai.com)

use std::time::Duration;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// Default timeout for LLM API calls.
const SEMANTIC_TIMEOUT: Duration = Duration::from_secs(120);

/// Default Anthropic API version header.
const ANTHROPIC_VERSION: &str = "2023-06-01";

// ---------------------------------------------------------------------------
// Provider detection
// ---------------------------------------------------------------------------

/// Which LLM provider to use for semantic checks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LlmProvider {
    Anthropic,
    OpenAi,
}

impl LlmProvider {
    /// Detect provider from environment.
    pub fn from_env() -> Self {
        match std::env::var("SYNODIC_LLM_PROVIDER")
            .unwrap_or_default()
            .to_lowercase()
            .as_str()
        {
            "openai" => LlmProvider::OpenAi,
            _ => LlmProvider::Anthropic,
        }
    }

    /// Get the credential from environment.
    pub fn credential(&self) -> Result<String> {
        let var = match self {
            LlmProvider::Anthropic => "ANTHROPIC_API_KEY",
            LlmProvider::OpenAi => "OPENAI_API_KEY",
        };
        std::env::var(var).with_context(|| format!("{var} not set"))
    }

    /// Get the API base URL.
    pub fn base_url(&self) -> String {
        match self {
            LlmProvider::Anthropic => std::env::var("ANTHROPIC_API_BASE")
                .unwrap_or_else(|_| "https://api.anthropic.com/v1".to_string()),
            LlmProvider::OpenAi => std::env::var("OPENAI_API_BASE")
                .unwrap_or_else(|_| "https://api.openai.com/v1".to_string()),
        }
    }
}

// ---------------------------------------------------------------------------
// Request / Response types
// ---------------------------------------------------------------------------

/// A simple LLM completion request.
#[derive(Debug, Clone)]
pub struct LlmRequest {
    /// System prompt.
    pub system: String,
    /// User message content.
    pub user_message: String,
    /// Model identifier (e.g. "claude-sonnet-4-20250514", "gpt-4o").
    pub model: String,
    /// Maximum tokens in the response.
    pub max_tokens: u32,
}

/// A simple LLM completion response.
#[derive(Debug, Clone)]
pub struct LlmResponse {
    /// The text content of the response.
    pub text: String,
}

// ---------------------------------------------------------------------------
// Anthropic API types
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct AnthropicRequest {
    model: String,
    max_tokens: u32,
    system: String,
    messages: Vec<AnthropicMessage>,
}

#[derive(Serialize)]
struct AnthropicMessage {
    role: String,
    content: String,
}

#[derive(Deserialize)]
struct AnthropicResponse {
    content: Vec<AnthropicContent>,
}

#[derive(Deserialize)]
struct AnthropicContent {
    text: Option<String>,
}

// ---------------------------------------------------------------------------
// OpenAI-compatible API types
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct OpenAiRequest {
    model: String,
    max_tokens: u32,
    messages: Vec<OpenAiMessage>,
}

#[derive(Serialize)]
struct OpenAiMessage {
    role: String,
    content: String,
}

#[derive(Deserialize)]
struct OpenAiResponse {
    choices: Vec<OpenAiChoice>,
}

#[derive(Deserialize)]
struct OpenAiChoice {
    message: OpenAiChoiceMessage,
}

#[derive(Deserialize)]
struct OpenAiChoiceMessage {
    content: Option<String>,
}

// ---------------------------------------------------------------------------
// Client
// ---------------------------------------------------------------------------

/// Lightweight LLM client for governance checks.
pub struct LlmClient {
    provider: LlmProvider,
    credential: String,
    base_url: String,
    client: reqwest::Client,
}

impl LlmClient {
    /// Create a new client, reading provider and credentials from env.
    pub fn from_env() -> Result<Self> {
        let provider = LlmProvider::from_env();
        let credential = provider.credential()?;
        let base_url = provider.base_url();
        let client = reqwest::Client::builder()
            .timeout(SEMANTIC_TIMEOUT)
            .build()
            .context("failed to build HTTP client")?;

        Ok(Self {
            provider,
            credential,
            base_url,
            client,
        })
    }

    /// The detected provider.
    pub fn provider(&self) -> &LlmProvider {
        &self.provider
    }

    /// Send a completion request and return the response text.
    pub async fn complete(&self, req: &LlmRequest) -> Result<LlmResponse> {
        match self.provider {
            LlmProvider::Anthropic => self.complete_anthropic(req).await,
            LlmProvider::OpenAi => self.complete_openai(req).await,
        }
    }

    async fn complete_anthropic(&self, req: &LlmRequest) -> Result<LlmResponse> {
        let url = format!("{}/messages", self.base_url);

        let body = AnthropicRequest {
            model: req.model.clone(),
            max_tokens: req.max_tokens,
            system: req.system.clone(),
            messages: vec![AnthropicMessage {
                role: "user".to_string(),
                content: req.user_message.clone(),
            }],
        };

        let resp = self
            .client
            .post(&url)
            .header("x-api-key", &self.credential)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .context("Anthropic API request failed")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Anthropic API returned {status}: {body}");
        }

        let parsed: AnthropicResponse = resp
            .json()
            .await
            .context("failed to parse Anthropic response")?;

        let text = parsed
            .content
            .into_iter()
            .filter_map(|c| c.text)
            .collect::<Vec<_>>()
            .join("");

        Ok(LlmResponse { text })
    }

    async fn complete_openai(&self, req: &LlmRequest) -> Result<LlmResponse> {
        let url = format!("{}/chat/completions", self.base_url);

        let body = OpenAiRequest {
            model: req.model.clone(),
            max_tokens: req.max_tokens,
            messages: vec![
                OpenAiMessage {
                    role: "system".to_string(),
                    content: req.system.clone(),
                },
                OpenAiMessage {
                    role: "user".to_string(),
                    content: req.user_message.clone(),
                },
            ],
        };

        let resp = self
            .client
            .post(&url)
            .header("authorization", format!("Bearer {}", self.credential))
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .context("OpenAI API request failed")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("OpenAI API returned {status}: {body}");
        }

        let parsed: OpenAiResponse = resp
            .json()
            .await
            .context("failed to parse OpenAI response")?;

        let text = parsed
            .choices
            .into_iter()
            .filter_map(|c| c.message.content)
            .collect::<Vec<_>>()
            .join("");

        Ok(LlmResponse { text })
    }
}

/// Default model for Anthropic semantic checks.
pub const DEFAULT_ANTHROPIC_MODEL: &str = "claude-sonnet-4-20250514";

/// Default model for OpenAI semantic checks.
pub const DEFAULT_OPENAI_MODEL: &str = "gpt-4o";

/// Get the default model for the current provider.
pub fn default_model_for_provider(provider: &LlmProvider) -> &'static str {
    match provider {
        LlmProvider::Anthropic => DEFAULT_ANTHROPIC_MODEL,
        LlmProvider::OpenAi => DEFAULT_OPENAI_MODEL,
    }
}
