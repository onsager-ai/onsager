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

use std::path::Path;

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

pub fn run() -> Result<()> {
    // Implementation lands in subsequent chunks: TS scanner, normalization
    // + allowlist, bidirectional comparison.
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
}
