//! API/UI contract enforcement (spec #151 Lever F).
//!
//! Asserts that the dashboard ↔ backend HTTP surface stays wired in both
//! directions:
//!
//! 1. Every backend route registered in stiglab + synodic has at least one
//!    dashboard caller, **or** sits on the [`EXTERNAL_ONLY_ROUTES`]
//!    allowlist with a reason — webhooks, OAuth callbacks, agent WS,
//!    dev-login, the governance proxy catchall, and bridge-debt
//!    redirects.
//! 2. Every backend path the dashboard calls (from
//!    `apps/dashboard/src/lib/api.ts` and `apps/dashboard/src/lib/sse.ts`)
//!    matches a route registered on a backend subsystem.
//!
//! Backed by static parsing — `syn` for the Rust route chains, a small
//! hand-rolled scanner for the TS string literals. No server boot, no
//! runtime dependency on the dashboard build.
//!
//! Pairs with `lint_seams` (Lever B) and the future `check-events` (Lever
//! E #150). Together they cover the three #131 contract surfaces:
//! subsystem-to-subsystem (B), event types (E), API/UI (F).

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use syn::visit::Visit;
use syn::{Expr, ExprLit, Lit};

/// One backend route registration extracted from a subsystem's source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackendRoute {
    /// The literal axum path, e.g. `"/api/workspaces/{id}/members"`.
    pub path: String,
    /// `"stiglab"` or `"synodic"`.
    pub subsystem: &'static str,
}

/// One dashboard call site — the path argument extracted from a
/// `request<T>(...)` or `EventSource(...)` invocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DashboardCall {
    /// Path as written in TS, e.g. `/workspaces/${encodeURIComponent(id)}`.
    pub path: String,
    /// Source file (workspace-relative).
    pub file: PathBuf,
    /// 1-based line number where the string literal opens.
    pub line: usize,
}

/// Walk `expr.route("...", ...)` method chains and collect the literal
/// path argument from each call. Anything we can't statically recognise
/// (computed paths, non-literal arguments) is skipped silently — those
/// would also defeat the matching logic, so flagging them here would just
/// be noise.
struct RouteVisitor {
    subsystem: &'static str,
    out: Vec<BackendRoute>,
}

impl<'ast> Visit<'ast> for RouteVisitor {
    fn visit_expr_method_call(&mut self, node: &'ast syn::ExprMethodCall) {
        // Recurse *first* so a chain `Router::new().route(A,_).route(B,_)`
        // records A before B (matching source order — the outermost call
        // is the deepest receiver).
        syn::visit::visit_expr_method_call(self, node);
        if node.method == "route" {
            if let Some(Expr::Lit(ExprLit {
                lit: Lit::Str(lit), ..
            })) = node.args.first()
            {
                self.out.push(BackendRoute {
                    path: lit.value(),
                    subsystem: self.subsystem,
                });
            }
        }
    }
}

/// Parse one Rust file and return every `.route("...", ...)` it registers.
pub fn parse_rust_routes(file: &Path, subsystem: &'static str) -> Result<Vec<BackendRoute>> {
    let source =
        std::fs::read_to_string(file).with_context(|| format!("read {}", file.display()))?;
    let ast = syn::parse_file(&source).with_context(|| format!("parse {}", file.display()))?;
    let mut visitor = RouteVisitor {
        subsystem,
        out: Vec::new(),
    };
    visitor.visit_file(&ast);
    Ok(visitor.out)
}

/// Scan a TypeScript source file and pull out the path argument from
/// every `request<T>(...)` and `EventSource(...)` call. Conservative —
/// only literal string forms are recognised; computed paths fall through
/// silently because they would defeat any matching anyway.
///
/// Backtick interpolations (`${...}`) are preserved verbatim — the
/// normaliser collapses them into `{x}` later so that
/// `/workspaces/${id}` and `/workspaces/{id}` (axum) compare equal.
pub fn parse_ts_calls(file: &Path) -> Result<Vec<DashboardCall>> {
    let source =
        std::fs::read_to_string(file).with_context(|| format!("read {}", file.display()))?;
    Ok(scan_ts_calls(&source, file))
}

fn scan_ts_calls(source: &str, file: &Path) -> Vec<DashboardCall> {
    let bytes = source.as_bytes();
    let n = bytes.len();
    let mut out = Vec::new();
    let mut i = 0;
    while i < n {
        // Comments and string literals never contain a call marker we
        // care about, so step over them wholesale rather than letting
        // the marker scan match commented-out or string-embedded code.
        if let Some(after) = skip_trivia_or_string(source, i) {
            i = after;
            continue;
        }
        if let Some(after_marker) = match_call_marker(bytes, i) {
            i = after_marker;
            if let Some((path, after_str, line)) = read_next_string(source, i) {
                out.push(DashboardCall {
                    path,
                    file: file.to_path_buf(),
                    line,
                });
                i = after_str;
                continue;
            }
        }
        i += 1;
    }
    out
}

/// Step past one comment or string literal starting at `i`. `None` if
/// the byte at `i` doesn't open one.
fn skip_trivia_or_string(source: &str, i: usize) -> Option<usize> {
    let bytes = source.as_bytes();
    let n = bytes.len();
    if i >= n {
        return None;
    }
    match bytes[i] {
        b'/' if i + 1 < n && bytes[i + 1] == b'/' => {
            let mut j = i + 2;
            while j < n && bytes[j] != b'\n' {
                j += 1;
            }
            Some(j)
        }
        b'/' if i + 1 < n && bytes[i + 1] == b'*' => {
            let mut j = i + 2;
            while j + 1 < n && !(bytes[j] == b'*' && bytes[j + 1] == b'/') {
                j += 1;
            }
            Some((j + 2).min(n))
        }
        b'\'' | b'"' | b'`' => read_next_string(source, i).map(|(_, end, _)| end),
        _ => None,
    }
}

/// Recognise the call-site markers we care about. Returns the byte
/// offset just past the opening `(` of the call's argument list, or
/// `None` if `i` doesn't start a marker.
fn match_call_marker(bytes: &[u8], i: usize) -> Option<usize> {
    // request<...>( — generic args may nest, so we count `<` vs `>`.
    if bytes[i..].starts_with(b"request<") {
        let mut j = i + b"request<".len();
        let mut depth: i32 = 1;
        while j < bytes.len() && depth > 0 {
            match bytes[j] {
                b'<' => depth += 1,
                b'>' => depth -= 1,
                _ => {}
            }
            j += 1;
        }
        // Skip any whitespace, then expect `(`.
        while j < bytes.len() && bytes[j].is_ascii_whitespace() {
            j += 1;
        }
        if j < bytes.len() && bytes[j] == b'(' {
            return Some(j + 1);
        }
        return None;
    }
    if bytes[i..].starts_with(b"EventSource(") {
        return Some(i + b"EventSource(".len());
    }
    None
}

/// Skip whitespace and `//` / `/* */` comments, then read one TS string
/// literal starting at the resulting position. Returns `(content,
/// next_offset, line_number)`. `None` if the next non-trivia byte isn't
/// a quote.
fn read_next_string(source: &str, mut i: usize) -> Option<(String, usize, usize)> {
    let bytes = source.as_bytes();
    let n = bytes.len();
    while i < n {
        match bytes[i] {
            b' ' | b'\t' | b'\n' | b'\r' => i += 1,
            b'/' if i + 1 < n && bytes[i + 1] == b'/' => {
                while i < n && bytes[i] != b'\n' {
                    i += 1;
                }
            }
            b'/' if i + 1 < n && bytes[i + 1] == b'*' => {
                i += 2;
                while i + 1 < n && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                    i += 1;
                }
                i = (i + 2).min(n);
            }
            _ => break,
        }
    }
    if i >= n {
        return None;
    }
    let quote = bytes[i];
    if quote != b'\'' && quote != b'"' && quote != b'`' {
        return None;
    }
    let line = source[..i].bytes().filter(|&b| b == b'\n').count() + 1;
    i += 1;
    let mut content = String::new();
    // Backtick template literals can embed `${...}` interpolations whose
    // bodies may contain matching `{}`, balanced strings, etc. We
    // don't evaluate them; we just preserve the raw `${...}` block so
    // the normaliser can map it to `{x}`.
    let mut tpl_depth: u32 = 0;
    while i < n {
        let c = bytes[i];
        if tpl_depth > 0 {
            content.push(c as char);
            if c == b'{' {
                tpl_depth += 1;
            } else if c == b'}' {
                tpl_depth -= 1;
            }
            i += 1;
            continue;
        }
        if c == b'\\' && i + 1 < n {
            content.push(c as char);
            content.push(bytes[i + 1] as char);
            i += 2;
            continue;
        }
        if c == quote {
            return Some((content, i + 1, line));
        }
        if quote == b'`' && c == b'$' && i + 1 < n && bytes[i + 1] == b'{' {
            content.push('$');
            content.push('{');
            tpl_depth = 1;
            i += 2;
            continue;
        }
        content.push(c as char);
        i += 1;
    }
    // Unterminated literal — bail out.
    None
}

// ---------------------------------------------------------------------------
// Normalization
// ---------------------------------------------------------------------------

/// Reduce a backend axum path to the comparison namespace shared with
/// dashboard calls.
///
/// - Strip the leading `/api/` (dashboard calls are written without it
///   since `API_BASE = '/api'`) or just the leading `/`.
/// - Replace path params `{name}` → `{x}`, catchall `{*name}` → `{*x}`.
/// - For synodic, prepend `governance/`. The dashboard reaches synodic
///   only via stiglab's `/api/governance/{*path}` proxy, so a synodic
///   route at `/foo` corresponds to the dashboard call `/governance/foo`.
pub fn normalize_backend(path: &str, subsystem: &str) -> String {
    let stripped = path
        .strip_prefix("/api/")
        .or_else(|| path.strip_prefix('/'))
        .unwrap_or(path);
    let scoped = if subsystem == "synodic" {
        format!("governance/{}", stripped)
    } else {
        stripped.to_string()
    };
    rewrite_path_params(&scoped)
}

/// Reduce a dashboard call path to the comparison namespace.
///
/// - `${scoped(...)}` and `${qs}` interpolations resolve to query
///   suffixes (`?workspace_id=...&...`). Replace them with `?` so
///   the trailing query gets stripped.
/// - Other `${...}` blocks are path-segment values — replace with `{x}`.
/// - Drop everything from the first `?` onward.
/// - Trim a leading `/`.
pub fn normalize_dashboard(path: &str) -> String {
    let mut s = String::new();
    let bytes = path.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'$' && i + 1 < bytes.len() && bytes[i + 1] == b'{' {
            let mut j = i + 2;
            let mut depth = 1usize;
            while j < bytes.len() && depth > 0 {
                match bytes[j] {
                    b'{' => depth += 1,
                    b'}' => depth -= 1,
                    _ => {}
                }
                j += 1;
            }
            let inside = path
                .get(i + 2..j.saturating_sub(1))
                .unwrap_or("")
                .trim();
            if inside.starts_with("scoped(") || inside == "qs" {
                s.push('?');
            } else {
                s.push_str("{x}");
            }
            i = j;
        } else {
            s.push(bytes[i] as char);
            i += 1;
        }
    }
    let s = s.split('?').next().unwrap_or("").trim_start_matches('/').to_string();
    s
}

/// Rewrite axum path params: `{name}` → `{x}`, `{*name}` → `{*x}`.
fn rewrite_path_params(path: &str) -> String {
    let mut out = String::new();
    let bytes = path.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'{' {
            let mut j = i + 1;
            while j < bytes.len() && bytes[j] != b'}' {
                j += 1;
            }
            let inside = path.get(i + 1..j).unwrap_or("");
            if inside.starts_with('*') {
                out.push_str("{*x}");
            } else {
                out.push_str("{x}");
            }
            i = (j + 1).min(bytes.len());
        } else {
            out.push(bytes[i] as char);
            i += 1;
        }
    }
    out.trim_start_matches('/').to_string()
}

/// Does a normalized dashboard call match a normalized backend route?
/// Exact equality wins; a backend ending in `{*x}` matches any path
/// that shares its prefix (axum catchall semantics).
pub fn matches_route(dashboard: &str, backend: &str) -> bool {
    if dashboard == backend {
        return true;
    }
    if let Some(prefix) = backend.strip_suffix("{*x}") {
        return dashboard.starts_with(prefix);
    }
    false
}

// ---------------------------------------------------------------------------
// Allowlist
// ---------------------------------------------------------------------------

/// Backend routes that legitimately have no `request<T>(...)` /
/// `EventSource(...)` caller in the dashboard. Each entry needs a
/// reason; the lint prints them on every run so the list stays
/// reviewable rather than invisible.
///
/// Entries here are matched against the **raw** axum path (before
/// normalization) so the file-grep stays trivial: an allowlist line
/// `/api/foo` lines up with the `.route("/api/foo", ...)` call in
/// `mod.rs`.
pub const EXTERNAL_ONLY_ROUTES: &[(&str, &str)] = &[
    (
        "/agent/ws",
        "agent worker WebSocket — agent binaries connect, not the dashboard",
    ),
    (
        "/api/auth/github",
        "OAuth start — entered via `<a href>` from LoginPage, not request<T>",
    ),
    (
        "/api/auth/github/callback",
        "OAuth callback — GitHub redirects the browser here",
    ),
    (
        "/api/auth/sso/redeem",
        "cross-environment SSO delegation — server-to-server, owner process only",
    ),
    (
        "/api/auth/sso/finish",
        "browser lands here after owner-side OAuth completes; no fetch",
    ),
    (
        "/api/github-app/install-start",
        "GitHub App install kickoff — `window.location.href` from WorkspaceCard, not request<T>",
    ),
    (
        "/api/github-app/callback",
        "GitHub App install callback — entered by GitHub redirect",
    ),
    (
        "/api/github-app/webhook",
        "forgiving alias for the GitHub webhook receiver — entered by GitHub",
    ),
    (
        "/api/webhooks/github",
        "GitHub webhook receiver for the workflow runtime",
    ),
    (
        "/webhooks/github",
        "portal webhook proxy — receives GitHub webhooks for the portal binary",
    ),
    (
        "/api/governance/{*path}",
        "catchall proxy that forwards to synodic; covered route-by-route via the synodic check",
    ),
    (
        "/api/tenants",
        "bridge-debt: 308 redirect to /api/workspaces (#173 — pending removal)",
    ),
    (
        "/api/tenants/{*rest}",
        "bridge-debt: 308 redirect to /api/workspaces/* (#173 — pending removal)",
    ),
    (
        "/health",
        "synodic internal health endpoint — ops-only, not a dashboard surface",
    ),
];

pub fn run() -> Result<()> {
    // Implementation lands in chunk 5: bidirectional comparison.
    println!("api-contract lint: skeleton (no checks wired yet — spec #151)");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_tmp(name: &str, body: &str) -> tempfile::NamedTempFile {
        let mut f = tempfile::Builder::new()
            .suffix(name)
            .tempfile()
            .expect("tempfile");
        f.write_all(body.as_bytes()).unwrap();
        f
    }

    #[test]
    fn extracts_route_paths_from_method_chain() {
        let src = r#"
            use axum::{routing::{get, post}, Router};
            fn build() -> Router {
                Router::new()
                    .route("/api/health", get(health))
                    .route("/api/workspaces/{id}", get(get_ws).delete(delete_ws))
            }
        "#;
        let f = write_tmp(".rs", src);
        let routes = parse_rust_routes(f.path(), "stiglab").unwrap();
        let paths: Vec<_> = routes.iter().map(|r| r.path.as_str()).collect();
        assert_eq!(paths, vec!["/api/health", "/api/workspaces/{id}"]);
        assert!(routes.iter().all(|r| r.subsystem == "stiglab"));
    }

    #[test]
    fn ignores_non_route_method_calls() {
        let src = r#"
            fn x() {
                let _ = vec.with_state(state);
                let _ = "/api/should-not-appear";
            }
        "#;
        let f = write_tmp(".rs", src);
        assert!(parse_rust_routes(f.path(), "stiglab").unwrap().is_empty());
    }

    #[test]
    fn parses_real_subsystem_sources() {
        // Smoke-test against the live source files. We don't pin exact
        // counts (those churn on every PR) — only that we extract
        // *something* and that the well-known anchors are present.
        let root = workspace_root();
        let stiglab =
            parse_rust_routes(&root.join("crates/stiglab/src/server/mod.rs"), "stiglab").unwrap();
        assert!(stiglab.len() > 20, "stiglab routes: {}", stiglab.len());
        let stiglab_paths: Vec<_> = stiglab.iter().map(|r| r.path.as_str()).collect();
        assert!(stiglab_paths.contains(&"/api/health"));
        assert!(stiglab_paths.contains(&"/api/workspaces"));

        let synodic =
            parse_rust_routes(&root.join("crates/synodic/src/cmd/serve.rs"), "synodic").unwrap();
        assert!(synodic.len() >= 5, "synodic routes: {}", synodic.len());
        let synodic_paths: Vec<_> = synodic.iter().map(|r| r.path.as_str()).collect();
        assert!(synodic_paths.contains(&"/health"));
        assert!(synodic_paths.contains(&"/events"));
    }

    fn workspace_root() -> std::path::PathBuf {
        // CARGO_MANIFEST_DIR points at xtask/; the workspace root is its parent.
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .to_path_buf()
    }

    #[test]
    fn skips_routes_with_non_literal_path() {
        let src = r#"
            fn x() {
                let path = "/api/dynamic";
                let _ = router.route(path, get(h));
                let _ = router.route("/api/static", get(h));
            }
        "#;
        let f = write_tmp(".rs", src);
        let routes = parse_rust_routes(f.path(), "stiglab").unwrap();
        let paths: Vec<_> = routes.iter().map(|r| r.path.as_str()).collect();
        assert_eq!(paths, vec!["/api/static"]);
    }

    fn scan(src: &str) -> Vec<String> {
        scan_ts_calls(src, Path::new("api.ts"))
            .into_iter()
            .map(|c| c.path)
            .collect()
    }

    #[test]
    fn extracts_path_from_request_call_with_generics() {
        let src = "
            const a = request<{ workspaces: Workspace[] }>('/workspaces');
            const b = request<{ session: Session }>(`/sessions/${id}`);
        ";
        assert_eq!(scan(src), vec!["/workspaces", "/sessions/${id}"]);
    }

    #[test]
    fn extracts_path_from_event_source_call() {
        let src = "const es = new EventSource(`/api/sessions/${sid}/logs`);";
        assert_eq!(scan(src), vec!["/api/sessions/${sid}/logs"]);
    }

    #[test]
    fn handles_multi_line_request_with_options() {
        let src = "
            request<{ ok: boolean }>(
              `/credentials/${name}`,
              { method: 'DELETE' },
            );
        ";
        assert_eq!(scan(src), vec!["/credentials/${name}"]);
    }

    #[test]
    fn preserves_template_interpolations_with_nested_calls() {
        let src = "
            request<X>(`/projects/${encodeURIComponent(id)}/issues${qs}`)
        ";
        assert_eq!(
            scan(src),
            vec!["/projects/${encodeURIComponent(id)}/issues${qs}"]
        );
    }

    #[test]
    fn ignores_non_call_string_literals() {
        let src = "
            const ONBOARDING = 'onsager.onboarding_seen';
            const url = `https://example.com/path`;
            // request<X>('not-a-call'); -- in a comment, scanner still
            // matches because it's not full TS, but a real api.ts has
            // none of these patterns. Exercised separately below.
        ";
        assert_eq!(scan(src), Vec::<String>::new());
    }

    #[test]
    fn parses_real_dashboard_api_ts() {
        let root = workspace_root();
        let calls = parse_ts_calls(&root.join("apps/dashboard/src/lib/api.ts")).unwrap();
        assert!(calls.len() > 30, "api.ts calls: {}", calls.len());
        let paths: Vec<_> = calls.iter().map(|c| c.path.as_str()).collect();
        assert!(paths.contains(&"/health"));
        assert!(paths.contains(&"/workspaces"));
        // sanity: every captured path starts with a slash
        for c in &calls {
            assert!(
                c.path.starts_with('/'),
                "non-path string captured: {:?}",
                c.path
            );
        }
    }

    #[test]
    fn parses_real_dashboard_sse_ts() {
        let root = workspace_root();
        let calls = parse_ts_calls(&root.join("apps/dashboard/src/lib/sse.ts")).unwrap();
        let paths: Vec<_> = calls.iter().map(|c| c.path.as_str()).collect();
        assert!(paths.iter().any(|p| p.contains("/sessions/") && p.ends_with("/logs")));
    }

    #[test]
    fn normalize_backend_strips_api_prefix_and_rewrites_params() {
        assert_eq!(normalize_backend("/api/health", "stiglab"), "health");
        assert_eq!(normalize_backend("/api/workspaces", "stiglab"), "workspaces");
        assert_eq!(
            normalize_backend("/api/workspaces/{id}/members", "stiglab"),
            "workspaces/{x}/members"
        );
        assert_eq!(
            normalize_backend("/api/governance/{*path}", "stiglab"),
            "governance/{*x}"
        );
        assert_eq!(normalize_backend("/agent/ws", "stiglab"), "agent/ws");
    }

    #[test]
    fn normalize_backend_scopes_synodic_under_governance() {
        assert_eq!(normalize_backend("/health", "synodic"), "governance/health");
        assert_eq!(normalize_backend("/events", "synodic"), "governance/events");
        assert_eq!(
            normalize_backend("/events/{id}/resolve", "synodic"),
            "governance/events/{x}/resolve"
        );
        assert_eq!(
            normalize_backend("/escalations/{id}/propose-resolution", "synodic"),
            "governance/escalations/{x}/propose-resolution"
        );
    }

    #[test]
    fn normalize_dashboard_collapses_interpolations() {
        assert_eq!(normalize_dashboard("/health"), "health");
        assert_eq!(normalize_dashboard("/sessions/${id}"), "sessions/{x}");
        assert_eq!(
            normalize_dashboard("/workspaces/${encodeURIComponent(id)}/members"),
            "workspaces/{x}/members"
        );
    }

    #[test]
    fn normalize_dashboard_drops_scoped_query_suffix() {
        assert_eq!(normalize_dashboard("/nodes${scoped(workspaceId)}"), "nodes");
        assert_eq!(
            normalize_dashboard("/governance/rules${scoped(workspaceId)}"),
            "governance/rules"
        );
        assert_eq!(
            normalize_dashboard(
                "/spine/events${scoped(workspaceId, { event_type: 'foo', limit: 10 })}"
            ),
            "spine/events"
        );
        assert_eq!(
            normalize_dashboard("/projects/${encodeURIComponent(id)}/issues${qs}"),
            "projects/{x}/issues"
        );
    }

    #[test]
    fn matches_route_handles_exact_and_catchall() {
        assert!(matches_route("workspaces", "workspaces"));
        assert!(matches_route("workspaces/{x}", "workspaces/{x}"));
        assert!(!matches_route("workspaces", "workspaces/{x}"));
        assert!(matches_route("governance/rules", "governance/{*x}"));
        assert!(matches_route(
            "governance/events/123/resolve",
            "governance/{*x}"
        ));
        assert!(!matches_route("workspaces", "governance/{*x}"));
    }

    #[test]
    fn allowlist_entries_are_unique_and_have_reasons() {
        let mut seen = std::collections::BTreeSet::new();
        for (path, reason) in EXTERNAL_ONLY_ROUTES {
            assert!(seen.insert(*path), "duplicate allowlist entry: {path}");
            assert!(!reason.trim().is_empty(), "missing reason for {path}");
        }
    }
}
