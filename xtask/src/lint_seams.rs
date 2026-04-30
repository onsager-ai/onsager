//! Architecture + bridge-pattern lints for the seam rule (ADR 0004 / spec #131
//! Lever B).
//!
//! What it checks (subsystem source = `crates/{forge,stiglab,synodic,ising}/`):
//!
//! 1. **Arch-deps** — subsystem A's `Cargo.toml` must not declare another
//!    subsystem as a path / git / version dep.
//! 2. **Sibling-subsystem HTTP** — references to a sibling subsystem's
//!    `*_URL` / `*_PORT` env var, or a `localhost:<port>` literal whose port
//!    matches another subsystem's well-known port, are flagged. Self-
//!    references are fine. The legitimate `reqwest::Client` callers in
//!    stiglab/synodic talk to GitHub / LLM APIs — those don't trip this lint.
//! 3. **`#[serde(alias = ...)]`** — bridges that ossify (PR #107 pattern).
//! 4. **`*_mirror.rs`** — files that mirror a spine concept into a private
//!    schema (PR #129 pattern, Lever D's removal target).
//! 5. **Legacy `pub type X = Y;`** — a type alias whose adjacent doc-comment
//!    contains "legacy" / "compat" / "deprecated" / "alias for" / "renamed".
//!
//! Producer-without-consumer is **gated on Lever E #150** — currently a
//! reminder, not a check.
//!
//! ## Escape hatch
//!
//! For a single line, prepend `// seam-allow: <non-empty reason>` on the
//! line immediately above. Reason text is mandatory and grep-able; CI shows
//! it in the lint output. Use sparingly — every allow is a legacy debt.
//! Cargo.toml violations have no escape hatch; arch-dep is hard-fail.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};

const SUBSYSTEMS: &[&str] = &["forge", "stiglab", "synodic", "ising"];

/// Well-known **default** ports each subsystem listens on (i.e. what
/// `cargo run -p <subsys> -- serve` binds without env overrides; verified
/// against the `*_PORT` defaults in each subsystem's serve command). Hard-
/// coded because they are part of the subsystem's external contract — see
/// root `CLAUDE.md`.
const SUBSYSTEM_PORTS: &[(&str, &str)] =
    &[("stiglab", "3000"), ("synodic", "3001"), ("forge", "3002")];

pub fn run() -> Result<()> {
    let root = workspace_root()?;
    let mut violations: Vec<Violation> = Vec::new();

    for subsys in SUBSYSTEMS {
        check_arch_deps(&root, subsys, &mut violations)?;
        let src = root.join("crates").join(subsys).join("src");
        if !src.is_dir() {
            continue;
        }
        for file in rust_files(&src)? {
            let name = file.file_name().and_then(|s| s.to_str()).unwrap_or("");
            if name.ends_with("_mirror.rs") {
                violations.push(Violation {
                    path: rel(&root, &file),
                    line: 1,
                    kind: "mirror-file",
                    message: format!(
                        "subsystem `{subsys}` defines `{name}` — mirror modules drift from the spine; collapse the schema with a discriminator and delete the file (Lever D)"
                    ),
                });
                // Per-file violation is enough; we still scan its contents
                // for any other lints below so they aren't masked.
            }
            check_file(&root, subsys, &file, &mut violations)?;
        }
    }

    // Workspace-wide: GitHub HTTP must only be constructed inside
    // `onsager-github`. Closes the "scattered GitHub call sites" drift
    // the new library is consolidating (#221, #220 Sub-issue A).
    check_github_http_wall(&root, &mut violations)?;

    let (kept, allowed) = apply_allow_list(violations);

    if !allowed.is_empty() {
        eprintln!("seam-allow exemptions used:");
        for (v, reason) in &allowed {
            eprintln!(
                "  {}:{} [{}] — allowed: {reason}",
                v.path.display(),
                v.line,
                v.kind
            );
        }
        eprintln!();
    }

    if !kept.is_empty() {
        eprintln!("seam-rule violations:");
        for v in &kept {
            eprintln!(
                "  {}:{} [{}] {}",
                v.path.display(),
                v.line,
                v.kind,
                v.message
            );
        }
        eprintln!();
        eprintln!("See ADR 0004 (docs/adr/0004-tighten-the-seams.md) and spec #131.");
        eprintln!(
            "To bypass a single line: prepend `// seam-allow: <reason>` immediately above it."
        );
        bail!(
            "seam-rule lint failed: {} violation(s) (excluding {} allowed)",
            kept.len(),
            allowed.len()
        );
    }

    println!(
        "seam-rule lint: clean (subsystems: {}; {} allowed exemption(s))",
        SUBSYSTEMS.join(", "),
        allowed.len()
    );
    eprintln!(
        "note: producer-without-consumer check is gated on Lever E (#150). \
         Until that lands, new event types still need a manual reviewer."
    );
    Ok(())
}

#[derive(Debug, Clone)]
struct Violation {
    path: PathBuf,
    line: usize,
    kind: &'static str,
    message: String,
}

fn workspace_root() -> Result<PathBuf> {
    let manifest = std::env::var("CARGO_MANIFEST_DIR")
        .context("CARGO_MANIFEST_DIR not set; run via `cargo run -p xtask`")?;
    Ok(Path::new(&manifest)
        .parent()
        .ok_or_else(|| anyhow!("xtask manifest has no parent"))?
        .to_path_buf())
}

fn rel(root: &Path, p: &Path) -> PathBuf {
    p.strip_prefix(root).unwrap_or(p).to_path_buf()
}

fn rust_files(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    walk(dir, &mut |p| {
        if p.extension().and_then(|s| s.to_str()) == Some("rs") {
            out.push(p.to_path_buf());
        }
    })?;
    out.sort();
    Ok(out)
}

fn walk(dir: &Path, visit: &mut dyn FnMut(&Path)) -> Result<()> {
    for entry in std::fs::read_dir(dir).with_context(|| format!("read_dir {}", dir.display()))? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            walk(&path, visit)?;
        } else {
            visit(&path);
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Cargo.toml: arch-deps lint
// ---------------------------------------------------------------------------

fn check_arch_deps(root: &Path, subsys: &str, out: &mut Vec<Violation>) -> Result<()> {
    let manifest = root.join("crates").join(subsys).join("Cargo.toml");
    if !manifest.is_file() {
        return Ok(());
    }
    let text = std::fs::read_to_string(&manifest)
        .with_context(|| format!("read {}", manifest.display()))?;
    let parsed: toml::Value =
        toml::from_str(&text).with_context(|| format!("parse {}", manifest.display()))?;

    let dep_tables = ["dependencies", "dev-dependencies", "build-dependencies"];

    // Top-level [dependencies] / [dev-dependencies] / [build-dependencies].
    for table_key in dep_tables {
        if let Some(table) = parsed.get(table_key).and_then(|v| v.as_table()) {
            for (name, _) in table {
                check_arch_dep_entry(root, &manifest, &text, subsys, name, table_key, None, out);
            }
        }
    }

    // Target-specific tables: [target.'<cfg>'.dependencies] / dev / build.
    // A subsystem-to-subsystem dep dressed up as cfg-gated would otherwise
    // bypass the lint.
    if let Some(target) = parsed.get("target").and_then(|v| v.as_table()) {
        for (cfg, value) in target {
            for table_key in dep_tables {
                if let Some(table) = value.get(table_key).and_then(|v| v.as_table()) {
                    for (name, _) in table {
                        check_arch_dep_entry(
                            root,
                            &manifest,
                            &text,
                            subsys,
                            name,
                            table_key,
                            Some(cfg.as_str()),
                            out,
                        );
                    }
                }
            }
        }
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn check_arch_dep_entry(
    root: &Path,
    manifest: &Path,
    text: &str,
    subsys: &str,
    name: &str,
    table_key: &str,
    cfg: Option<&str>,
    out: &mut Vec<Violation>,
) {
    if name == subsys {
        return;
    }
    if !SUBSYSTEMS.contains(&name) {
        return;
    }
    let line = find_dep_line(text, table_key, name, cfg);
    let table_label = match cfg {
        Some(c) => format!("target.{c}.{table_key}"),
        None => table_key.to_string(),
    };
    out.push(Violation {
        path: rel(root, manifest),
        line,
        kind: "arch-dep",
        message: format!(
            "subsystem `{subsys}` declares `{name}` as [{table_label}] — subsystems must not depend on each other (ADR 0001)"
        ),
    });
}

/// Find the 1-indexed line where a dep declaration starts. Best-effort: we
/// look for a `<name>\s*=` line after the matching `[<table>]` header. For
/// target-specific tables (`cfg = Some(...)`), the header is matched on
/// suffix (`.{table}`) since the cfg literal can be quoted multiple ways.
fn find_dep_line(text: &str, table: &str, name: &str, cfg: Option<&str>) -> usize {
    let plain_header = format!("[{table}]");
    let target_suffix = format!(".{table}]");
    let mut in_section = false;
    for (i, line) in text.lines().enumerate() {
        let t = line.trim_start();
        if t.starts_with('[') && t.ends_with(']') {
            in_section = match cfg {
                None => t == plain_header,
                Some(_) => t.starts_with("[target.") && t.ends_with(target_suffix.as_str()),
            };
            continue;
        }
        if in_section {
            let rest = t.split_once('=').map(|(k, _)| k.trim()).unwrap_or("");
            if rest == name {
                return i + 1;
            }
        }
    }
    1
}

// ---------------------------------------------------------------------------
// Per-file scans
// ---------------------------------------------------------------------------

fn check_file(root: &Path, subsys: &str, path: &Path, out: &mut Vec<Violation>) -> Result<()> {
    let text = std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    // Split once and reuse — `check_legacy_type_alias` needs the full
    // line context for its doc-comment look-back, and re-collecting per
    // line would be O(n²) on files with many `pub type` declarations.
    let lines: Vec<&str> = text.lines().collect();
    for (idx, line) in lines.iter().enumerate() {
        let n = idx + 1;
        check_sibling_url(root, subsys, path, n, line, out);
        check_serde_alias(root, path, n, line, out);
        check_legacy_type_alias(root, path, n, line, &lines, out);
    }
    Ok(())
}

// ---- sibling-subsystem URL / env-var ------------------------------------

fn check_sibling_url(
    root: &Path,
    subsys: &str,
    path: &Path,
    line_no: usize,
    line: &str,
    out: &mut Vec<Violation>,
) {
    // Strip a trailing line comment so a `// seam-allow:` (or any other
    // textual mention of the env var inside a comment) doesn't itself trip.
    let code = strip_line_comment(line);

    for sibling in SUBSYSTEMS {
        if *sibling == subsys {
            continue;
        }
        let upper = sibling.to_uppercase();
        let url_var = format!("{upper}_URL");
        let port_var = format!("{upper}_PORT");
        let host = format!("{sibling}:");
        if code.contains(&url_var) || code.contains(&port_var) {
            out.push(Violation {
                path: rel(root, path),
                line: line_no,
                kind: "sibling-env",
                message: format!(
                    "subsystem `{subsys}` references sibling `{sibling}` via `{url_var}`/`{port_var}` — that's a direct HTTP RPC to another subsystem; coordinate via the spine instead (Lever C / ADR 0001)"
                ),
            });
            continue;
        }
        // Only flag URL-shaped references: `http://forge:` / `https://forge:`.
        // Bare `"forge:..."` strings are commonly stream IDs / tool names, not
        // hostnames, and would generate noise.
        if code.contains(&format!("http://{host}")) || code.contains(&format!("https://{host}")) {
            out.push(Violation {
                path: rel(root, path),
                line: line_no,
                kind: "sibling-host",
                message: format!(
                    "subsystem `{subsys}` constructs an HTTP URL targeting sibling host `{sibling}` — coordinate via the spine instead (Lever C)"
                ),
            });
            continue;
        }
    }

    if let Some(port) = sibling_localhost_port(code, subsys) {
        out.push(Violation {
            path: rel(root, path),
            line: line_no,
            kind: "sibling-port",
            message: format!(
                "subsystem `{subsys}` references `localhost:{port}` — that's another subsystem's well-known port; coordinate via the spine instead (Lever C)"
            ),
        });
    }
}

fn sibling_localhost_port(line: &str, subsys: &str) -> Option<&'static str> {
    for (sibling, port) in SUBSYSTEM_PORTS {
        if *sibling == subsys {
            continue;
        }
        let needle = format!("localhost:{port}");
        if line.contains(&needle) {
            return Some(port);
        }
    }
    None
}

/// Strip a trailing `//` line comment, but **only** when the `//` is outside
/// a string literal. Without this, `"http://host"` in a real URL is treated
/// as a comment start and the rest of the line vanishes — which is exactly
/// the case the sibling-host lint cares about.
///
/// Doesn't handle raw strings (`r"..."` / `r#"..."#`); those are rare in
/// lint targets and the worst case is a few extra characters preserved.
fn strip_line_comment(line: &str) -> &str {
    let bytes = line.as_bytes();
    let mut in_str = false;
    let mut prev_backslash = false;
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if in_str {
            if c == b'\\' && !prev_backslash {
                prev_backslash = true;
            } else {
                if c == b'"' && !prev_backslash {
                    in_str = false;
                }
                prev_backslash = false;
            }
            i += 1;
            continue;
        }
        if c == b'"' {
            in_str = true;
        } else if c == b'/' && bytes.get(i + 1) == Some(&b'/') {
            return &line[..i];
        }
        i += 1;
    }
    line
}

// ---- serde(alias = ...) -------------------------------------------------

fn check_serde_alias(
    root: &Path,
    path: &Path,
    line_no: usize,
    line: &str,
    out: &mut Vec<Violation>,
) {
    let code = strip_line_comment(line);
    if code.contains("serde(alias") || code.contains("serde( alias") {
        out.push(Violation {
            path: rel(root, path),
            line: line_no,
            kind: "serde-alias",
            message: "`#[serde(alias = ...)]` ossifies; renames must land atomically — drop the alias and update call sites in the same PR (PR #107 drift pattern)".to_string(),
        });
    }
}

// ---- legacy `pub type X = Y;` -------------------------------------------

fn check_legacy_type_alias(
    root: &Path,
    path: &Path,
    line_no: usize,
    line: &str,
    lines: &[&str],
    out: &mut Vec<Violation>,
) {
    let trimmed = line.trim_start();
    if !trimmed.starts_with("pub type ") {
        return;
    }
    // Heuristic: look at up to the previous 5 lines for a doc-comment
    // containing one of the legacy markers. `lines` is split once per file
    // by `check_file` and reused across all violations in the same file.
    let start = line_no.saturating_sub(5).max(1);
    let mut legacy = false;
    for back in start..line_no {
        let prev = lines.get(back - 1).copied().unwrap_or("");
        let s = prev.trim_start();
        if !s.starts_with("///") && !s.starts_with("//") {
            continue;
        }
        let lower = s.to_lowercase();
        if lower.contains("legacy")
            || lower.contains("deprecated")
            || lower.contains("compat")
            || lower.contains("alias for ")
            || lower.contains("renamed")
            || lower.contains("for backwards")
            || lower.contains("until ")
        {
            legacy = true;
            break;
        }
    }
    if legacy {
        out.push(Violation {
            path: rel(root, path),
            line: line_no,
            kind: "legacy-type-alias",
            message: format!(
                "`{}` looks like a legacy/compat type alias — renames land in one PR; drop the alias and update call sites (PR #107 drift pattern)",
                trimmed.trim_end_matches(';')
            ),
        });
    }
}

// ---------------------------------------------------------------------------
// Allow-list
// ---------------------------------------------------------------------------

/// Apply the `// seam-allow: <reason>` escape.
///
/// - **Line-level violations** (everything except `mirror-file`): a violation
///   at line N is suppressed if line N-1 (after trimming) starts with
///   `// seam-allow:` and has a non-empty reason. A violation on line 1
///   has no "line above" and therefore **cannot** be allow-listed this way.
/// - **File-level violations** (`mirror-file`): the marker must be the
///   file's first non-empty line — a single `// seam-allow: <reason>`
///   header that documents the whole file's exemption.
///
/// Cargo.toml violations (`arch-dep`) are not exempt-able and always pass
/// through (they're hard-fail per ADR 0004).
fn apply_allow_list(violations: Vec<Violation>) -> (Vec<Violation>, Vec<(Violation, String)>) {
    // Group by file to avoid re-reading.
    let mut by_file: std::collections::BTreeMap<PathBuf, Vec<Violation>> = Default::default();
    let mut cargo: Vec<Violation> = Vec::new();
    for v in violations {
        if v.kind == "arch-dep" {
            cargo.push(v);
        } else {
            by_file.entry(v.path.clone()).or_default().push(v);
        }
    }

    let mut kept: Vec<Violation> = cargo;
    let mut allowed: Vec<(Violation, String)> = Vec::new();
    let mut seen_pairs: BTreeSet<(PathBuf, usize, &'static str)> = BTreeSet::new();

    for (path, vs) in by_file {
        let abs = std::env::var("CARGO_MANIFEST_DIR")
            .ok()
            .and_then(|d| Path::new(&d).parent().map(|p| p.join(&path)))
            .unwrap_or_else(|| path.clone());
        let text = std::fs::read_to_string(&abs).unwrap_or_default();
        let lines: Vec<&str> = text.lines().collect();

        for v in vs {
            // Dedupe identical (path, line, kind) — multiple checks may flag
            // the same construct.
            let key = (v.path.clone(), v.line, v.kind);
            if !seen_pairs.insert(key) {
                continue;
            }
            let marker = if v.kind == "mirror-file" {
                // File-level violation: the marker is a single header line
                // at the top of the file. We accept the first non-empty
                // line of the file as the marker.
                lines
                    .iter()
                    .find(|l| !l.trim().is_empty())
                    .copied()
                    .unwrap_or("")
                    .trim_start()
            } else if v.line > 1 {
                lines.get(v.line - 2).copied().unwrap_or("").trim_start()
            } else {
                // Line-level violation on line 1 has no line above; the
                // "// seam-allow:" rule cannot apply.
                ""
            };
            if let Some(reason) = marker.strip_prefix("// seam-allow:") {
                let reason = reason.trim();
                if !reason.is_empty() {
                    allowed.push((v, reason.to_string()));
                    continue;
                }
            }
            kept.push(v);
        }
    }
    (kept, allowed)
}

// ---------------------------------------------------------------------------
// Workspace-wide: GitHub HTTP wall (#221 / #220 Sub-issue A)
// ---------------------------------------------------------------------------
//
// The `onsager-github` library is the only place that should construct
// HTTP clients aimed at `api.github.com` / `github.com`. Every other
// crate must go through the library — that's the "stop the bleeding"
// move from #221, and it's load-bearing for #150's adapter registry
// and #220's edge-subsystem promotion.
//
// We flag string literals containing the canonical GitHub host
// substrings rather than `reqwest::Client::new()` directly, because
// reqwest has plenty of legitimate non-GitHub callers (Anthropic API,
// SSO endpoints, etc.). Hostname literals are precise.

const GITHUB_HOST_NEEDLES: &[&str] = &[
    "api.github.com",
    "github.com/login/oauth",
    "github.com/app/installations",
];

/// Crates whose `src/` may legitimately mention GitHub host literals.
/// The library itself owns the wall.
const GITHUB_HTTP_WALL_ALLOWED_CRATES: &[&str] = &["onsager-github"];

/// Files known to still construct GitHub HTTP outside the library,
/// scheduled for migration in a follow-up under #220 Sub-issue A.
/// Each entry is `(relative-path, reason)`. The reason is grep-able
/// and printed at every CI run so the debt stays visible. Remove an
/// entry when its migration PR lands.
const GITHUB_HTTP_WALL_PENDING_FILES: &[(&str, &str)] = &[
    (
        "crates/stiglab/src/server/routes/projects.rs",
        "list_recent_issues / list_recent_pulls / get_issue / get_pull-style reads — fold into onsager-github::api::{issues,pulls} with the workspace ↔ portal split (#220 Sub-issue B)",
    ),
    (
        "crates/stiglab/src/server/workflow_activation.rs",
        "ensure_label / register_webhook / deregister_webhook — fold into onsager-github::api::{labels,webhooks} once the workflow URL-resolution + ActivationError shape is decoupled (follow-up to #221)",
    ),
];

fn check_github_http_wall(root: &Path, out: &mut Vec<Violation>) -> Result<()> {
    let crates_dir = root.join("crates");
    if !crates_dir.is_dir() {
        return Ok(());
    }
    let mut pending_hits: BTreeSet<&'static str> = BTreeSet::new();
    for entry in std::fs::read_dir(&crates_dir)? {
        let entry = entry?;
        let crate_dir = entry.path();
        if !crate_dir.is_dir() {
            continue;
        }
        let crate_name = match crate_dir.file_name().and_then(|s| s.to_str()) {
            Some(n) => n,
            None => continue,
        };
        if GITHUB_HTTP_WALL_ALLOWED_CRATES.contains(&crate_name) {
            continue;
        }
        let src = crate_dir.join("src");
        if !src.is_dir() {
            continue;
        }
        for file in rust_files(&src)? {
            let rel_path = rel(root, &file);
            let rel_str = rel_path.to_string_lossy().replace('\\', "/");
            let pending = GITHUB_HTTP_WALL_PENDING_FILES
                .iter()
                .find(|(p, _)| *p == rel_str.as_str());
            let text = std::fs::read_to_string(&file)
                .with_context(|| format!("read {}", file.display()))?;
            for (idx, line) in text.lines().enumerate() {
                let code = strip_line_comment(line);
                for needle in GITHUB_HOST_NEEDLES {
                    if code.contains(needle) {
                        if let Some((path, _)) = pending {
                            pending_hits.insert(*path);
                        } else {
                            out.push(Violation {
                                path: rel_path.clone(),
                                line: idx + 1,
                                kind: "github-http-wall",
                                message: format!(
                                    "crate `{crate_name}` references `{needle}` — GitHub HTTP must go through `onsager-github` (#221)"
                                ),
                            });
                        }
                    }
                }
            }
        }
    }
    if !pending_hits.is_empty() {
        eprintln!("github-http-wall pending migrations (follow-up to #221):");
        for (path, reason) in GITHUB_HTTP_WALL_PENDING_FILES {
            if pending_hits.contains(path) {
                eprintln!("  {path} — {reason}");
            }
        }
        eprintln!();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_path() -> &'static Path {
        Path::new("crates/forge/src/test.rs")
    }

    fn fake_root() -> &'static Path {
        Path::new("/")
    }

    #[test]
    fn flags_sibling_env_var() {
        let mut v = Vec::new();
        check_sibling_url(
            fake_root(),
            "forge",
            fake_path(),
            10,
            r#"std::env::var("STIGLAB_URL")"#,
            &mut v,
        );
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].kind, "sibling-env");
    }

    #[test]
    fn flags_sibling_url_in_string() {
        let mut v = Vec::new();
        check_sibling_url(
            fake_root(),
            "forge",
            fake_path(),
            10,
            r#"let url = "http://stiglab:3000/api/x";"#,
            &mut v,
        );
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].kind, "sibling-host");
    }

    #[test]
    fn ignores_self_env_var() {
        let mut v = Vec::new();
        check_sibling_url(
            fake_root(),
            "forge",
            fake_path(),
            10,
            r#"std::env::var("FORGE_URL")"#,
            &mut v,
        );
        assert!(v.is_empty(), "self-references must not fire: {:?}", v);
    }

    #[test]
    fn ignores_stream_id_with_colon() {
        // PR #131 false-positive regression: `format!("forge:{id}")` is a
        // stream key, not a hostname. Tightened heuristic only matches URLs.
        let mut v = Vec::new();
        check_sibling_url(
            fake_root(),
            "stiglab",
            fake_path(),
            10,
            r#"format!("forge:{artifact_id}")"#,
            &mut v,
        );
        assert!(v.is_empty(), "stream ids must not fire: {:?}", v);
    }

    #[test]
    fn ignores_event_name_with_dot() {
        let mut v = Vec::new();
        check_sibling_url(
            fake_root(),
            "stiglab",
            fake_path(),
            10,
            r#"emit("forge.gate_requested", payload)"#,
            &mut v,
        );
        assert!(v.is_empty(), "event names must not fire: {:?}", v);
    }

    #[test]
    fn flags_localhost_sibling_port() {
        let mut v = Vec::new();
        check_sibling_url(
            fake_root(),
            "forge",
            fake_path(),
            10,
            r#"let s = "http://localhost:3000";"#,
            &mut v,
        );
        // Either sibling-host (URL form) or sibling-port (localhost form) is
        // acceptable — both indicate the same problem.
        assert_eq!(v.len(), 1);
        assert!(matches!(v[0].kind, "sibling-host" | "sibling-port"));
    }

    #[test]
    fn flags_serde_alias() {
        let mut v = Vec::new();
        check_serde_alias(
            fake_root(),
            fake_path(),
            10,
            r#"#[serde(alias = "old_name")]"#,
            &mut v,
        );
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].kind, "serde-alias");
    }

    #[test]
    fn ignores_serde_default() {
        let mut v = Vec::new();
        check_serde_alias(fake_root(), fake_path(), 10, r#"#[serde(default)]"#, &mut v);
        assert!(v.is_empty());
    }

    #[test]
    fn flags_legacy_type_alias_with_doc_keyword() {
        let mut v = Vec::new();
        let lines = ["/// Legacy alias for the new name.", "pub type Old = New;"];
        check_legacy_type_alias(fake_root(), fake_path(), 2, lines[1], &lines, &mut v);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].kind, "legacy-type-alias");
    }

    #[test]
    fn ignores_plain_type_alias() {
        let mut v = Vec::new();
        let lines = [
            "/// A reusable shorthand for a complex generic.",
            "pub type Result<T> = std::result::Result<T, Error>;",
        ];
        check_legacy_type_alias(fake_root(), fake_path(), 2, lines[1], &lines, &mut v);
        assert!(
            v.is_empty(),
            "non-legacy aliases must not fire: {:?}",
            v.iter().map(|x| &x.message).collect::<Vec<_>>()
        );
    }

    #[test]
    fn allow_list_no_marker_above_line_1_for_line_violation() {
        // For line-level violations on line 1 there is no "line above";
        // the seam-allow rule must not silently match against line 1 itself.
        let lines = ["// seam-allow: should NOT apply to a line-1 violation"];
        let v_line: usize = 1;
        let marker = if v_line > 1 {
            lines.get(v_line - 2).copied().unwrap_or("")
        } else {
            ""
        };
        assert!(
            marker.is_empty(),
            "line-1 line-violations must have no marker"
        );
    }

    #[test]
    fn allow_list_uses_first_line_for_mirror_file() {
        // mirror-file violations are file-level; the marker is the first
        // non-empty line of the file.
        let lines = [
            "// seam-allow: collapsed in Lever D (#149)",
            "//! file docs",
            "use anyhow::Result;",
        ];
        let marker = lines
            .iter()
            .find(|l| !l.trim().is_empty())
            .copied()
            .unwrap_or("")
            .trim_start();
        assert!(marker.starts_with("// seam-allow:"));
    }

    #[test]
    fn allow_list_suppresses_with_reason() {
        // A violation at line 4 should look at lines.get(2) — i.e. line 3
        // (1-indexed). That's where we put the `// seam-allow:` marker.
        let lines = [
            "// preamble",            // line 1
            "fn before() {}",         // line 2
            "// seam-allow: legacy",  // line 3 — the marker
            "let x = STIGLAB_URL();", // line 4 — the violation
        ];
        let violation_line: usize = 4;
        let prev_idx = violation_line.saturating_sub(2);
        let prev = lines.get(prev_idx).copied().unwrap_or("").trim_start();
        assert!(prev.starts_with("// seam-allow:"), "got {prev:?}");
        let reason = prev.strip_prefix("// seam-allow:").unwrap().trim();
        assert!(!reason.is_empty());
    }

    #[test]
    fn allow_list_rejects_empty_reason() {
        let bad = "// seam-allow:";
        let reason = bad.strip_prefix("// seam-allow:").unwrap().trim();
        assert!(reason.is_empty(), "empty-reason allow must NOT pass");
    }
}
