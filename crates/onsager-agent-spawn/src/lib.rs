//! Shared agent-session spawn wiring (spec #535, Part of #533).
//!
//! The constants and JSON builders below were authored inline in
//! stiglab's `agent/session/process.rs` by spec #530/#532. Per #533 the
//! production scheduler-side runner (`ClaudeCliRunner`) must spawn the
//! same `claude` CLI with the same MCP-server / liveness-Stop-hook
//! wiring — but stiglab and the scheduler live on opposite sides of the
//! subsystem seam and cannot import each other. This crate is the pure
//! utility both depend on instead: extracted **verbatim** from stiglab,
//! it carries no IO, no env reads, and no subsystem knowledge, so
//! sharing it across the seam is seam-correct (a utility crate is not a
//! subsystem).
//!
//! Behavior is unchanged from the stiglab originals; the unit tests
//! below are the parity assertions #532 added, moved with the functions.

/// Env var (read on the agent node) holding the base URL of portal's
/// liveness endpoint, e.g.
/// `https://app.example.com/api/internal/session-liveness`. Unset → no
/// Stop hook is configured (fail-safe default for local dev and tests);
/// the authoritative gate (#523) alone then covers liveness.
pub const LIVENESS_HOOK_URL_ENV: &str = "ONSAGER_LIVENESS_HOOK_URL";

/// Env var holding the session's workspace-scoped bearer token. The
/// Stop hook interpolates it into the `Authorization` header at call
/// time (kept out of the on-disk settings via Claude Code's
/// `allowedEnvVars`); the MCP server config (spec #531) interpolates the
/// same var into its `Authorization` header. Provisioned into the session
/// env by stiglab's dispatch path (decrypted from the
/// `portal.session_requested` event); when absent the Stop hook's request
/// is unauthorized → `401` → it fails open (the authoritative gate #523
/// still applies) and no MCP server is registered.
pub const SESSION_TOKEN_ENV: &str = "ONSAGER_SESSION_TOKEN";

/// Env var (read on the agent node) holding portal's MCP endpoint URL,
/// e.g. `https://app.example.com/mcp/messages`. Unset → no MCP server is
/// registered (the agent can't emit via MCP, but the session still runs).
/// Spec #531.
pub const MCP_URL_ENV: &str = "ONSAGER_MCP_URL";

/// Build the inline `--settings` JSON that registers the liveness Stop
/// hook. Pure (no env, no IO) so the wire shape is unit-testable.
///
/// The hook is an **HTTP** hook (Claude Code `>= 2.1.x`): on every stop
/// attempt it POSTs to the manifest evaluator, which returns allow
/// (empty 2xx) or `{ "decision": "block", "reason": ... }`. Claude Code
/// fails the hook open on any non-2xx / timeout. Consecutive blocks are
/// capped by `CLAUDE_CODE_STOP_HOOK_BLOCK_CAP` (default 8); the decision
/// is recomputed from the manifest on every call, so a session that is
/// still empty stays blocked until the cap — we never short-circuit on
/// a `stop_hook_active`-style flag.
///
/// Our stiglab `session_id` + `workspace_id` are baked into the URL
/// query string (the POST body carries Claude Code's *own* session id,
/// which is unrelated to our manifest stream key).
pub fn stop_hook_settings_json(
    hook_url_base: &str,
    workspace_id: &str,
    session_id: &str,
) -> String {
    // Choose `?` vs `&` so a base URL that already carries a query
    // string (or a trailing `?`) stays a valid URL rather than
    // producing a double `?`.
    let sep = if hook_url_base.contains('?') {
        '&'
    } else {
        '?'
    };
    let url = format!(
        "{hook_url_base}{sep}workspace_id={}&session_id={}",
        urlencode(workspace_id),
        urlencode(session_id),
    );
    let token_ref = format!("Bearer ${SESSION_TOKEN_ENV}");
    serde_json::json!({
        "hooks": {
            "Stop": [
                {
                    "hooks": [
                        {
                            "type": "http",
                            "url": url,
                            // Short timeout: a network blip should fail
                            // the hook open promptly, not stall the stop
                            // for the 600s HTTP-hook default.
                            "timeout": 10,
                            "headers": { "Authorization": token_ref },
                            "allowedEnvVars": [SESSION_TOKEN_ENV],
                        }
                    ]
                }
            ]
        }
    })
    .to_string()
}

/// Minimal percent-encoding for query-string values. Server-minted ids
/// (`ws_<ulid>`, ULID/UUID session ids) are already URL-safe; this is
/// defensive against any reserved character sneaking in.
pub fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Build the inline `--mcp-config` JSON registering portal's MCP server
/// (spec #531) so the agent can call `emit_artifact` / `declare_no_output`
/// / `read_emit_status`. Pure so the wire shape is unit-testable.
///
/// The bearer token is interpolated from `$ONSAGER_SESSION_TOKEN` at
/// startup (Claude Code expands `${VAR}` in MCP `headers`). This is only
/// wired when the token is actually present in the env — Claude Code
/// **fails to parse** an MCP config whose `${VAR}` is unset, so registering
/// it tokenless would break the whole session rather than degrade.
pub fn mcp_config_json(mcp_url: &str) -> String {
    serde_json::json!({
        "mcpServers": {
            "onsager": {
                "type": "http",
                "url": mcp_url,
                "headers": {
                    "Authorization": format!("Bearer ${{{SESSION_TOKEN_ENV}}}"),
                },
            }
        }
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stop_hook_settings_have_http_hook_shape() {
        let json = stop_hook_settings_json(
            "https://app.example.com/api/internal/session-liveness",
            "ws_123",
            "sess_abc",
        );
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        let hook = &v["hooks"]["Stop"][0]["hooks"][0];
        assert_eq!(hook["type"], "http");
        // Identity rides on the URL query string, not the POST body
        // (the body carries Claude Code's own session id).
        let url = hook["url"].as_str().unwrap();
        assert!(url.starts_with("https://app.example.com/api/internal/session-liveness?"));
        assert!(url.contains("workspace_id=ws_123"));
        assert!(url.contains("session_id=sess_abc"));
        // Token is interpolated from env at call time, never inlined.
        assert_eq!(
            hook["headers"]["Authorization"],
            "Bearer $ONSAGER_SESSION_TOKEN"
        );
        assert_eq!(hook["allowedEnvVars"][0], "ONSAGER_SESSION_TOKEN");
        // Short timeout so a blip fails open promptly.
        assert_eq!(hook["timeout"], 10);
    }

    #[test]
    fn stop_hook_url_uses_amp_when_base_already_has_query() {
        // A base URL carrying its own query string must stay valid —
        // append with `&`, not a second `?`.
        let json = stop_hook_settings_json("https://h.example/live?env=prod", "ws_1", "s_1");
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        let url = v["hooks"]["Stop"][0]["hooks"][0]["url"].as_str().unwrap();
        assert_eq!(
            url,
            "https://h.example/live?env=prod&workspace_id=ws_1&session_id=s_1"
        );
        // Exactly one `?` in the final URL.
        assert_eq!(url.matches('?').count(), 1);
    }

    #[test]
    fn mcp_config_registers_http_server_with_interpolated_token() {
        let json = mcp_config_json("https://app.example.com/mcp/messages");
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        let server = &v["mcpServers"]["onsager"];
        assert_eq!(server["type"], "http");
        assert_eq!(server["url"], "https://app.example.com/mcp/messages");
        // Token interpolated from env at startup, never inlined.
        assert_eq!(
            server["headers"]["Authorization"],
            "Bearer ${ONSAGER_SESSION_TOKEN}"
        );
    }

    #[test]
    fn urlencode_passes_safe_ids_and_escapes_reserved() {
        assert_eq!(urlencode("ws_01HZX-abc.def~9"), "ws_01HZX-abc.def~9");
        assert_eq!(urlencode("a b&c=d"), "a%20b%26c%3Dd");
    }
}
