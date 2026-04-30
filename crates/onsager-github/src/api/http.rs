//! Shared HTTP client for GitHub API calls.
//!
//! Single process-wide `reqwest::Client` so connection pools + TLS
//! state get reused across JWT minting, installation tokens, paginated
//! reads, and webhook side-effects. **This is the only place in the
//! workspace that should construct a `reqwest::Client` aimed at GitHub
//! API endpoints** — the `xtask lint-seams` rule enforces that wall.

use std::sync::OnceLock;

/// Base URL for the public GitHub REST API.
pub const GITHUB_API: &str = "https://api.github.com";

/// User-Agent string sent on every API call. GitHub requires a
/// non-empty UA; setting one here keeps the value consistent across
/// callers (today stiglab and portal had divergent UAs).
pub const USER_AGENT: &str = "onsager-github/0.1";

/// Shared GitHub HTTP client. First call builds it; subsequent calls
/// reuse the same `Arc`-cloned handle.
pub fn client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .build()
            .expect("reqwest client must build")
    })
}
