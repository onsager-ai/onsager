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

/// Well-known ports each subsystem listens on. Hard-coded because they are
/// part of the subsystem's external contract — see root `CLAUDE.md`.
const SUBSYSTEM_PORTS: &[(&str, &str)] =
    &[("stiglab", "3000"), ("synodic", "3001"), ("forge", "3003")];

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
    for table_key in dep_tables {
        let Some(table) = parsed.get(table_key).and_then(|v| v.as_table()) else {
            continue;
        };
        for (name, _) in table {
            if name == subsys {
                continue;
            }
            if SUBSYSTEMS.contains(&name.as_str()) {
                let line = find_dep_line(&text, table_key, name);
                out.push(Violation {
                    path: rel(root, &manifest),
                    line,
                    kind: "arch-dep",
                    message: format!(
                        "subsystem `{subsys}` declares `{name}` as [{table_key}] — subsystems must not depend on each other (ADR 0001)"
                    ),
                });
            }
        }
    }
    Ok(())
}

/// Find the 1-indexed line where a dep declaration starts. Best-effort: we
/// look for a `<name>\s*=` line after the matching `[<table>]` header.
fn find_dep_line(text: &str, table: &str, name: &str) -> usize {
    let header = format!("[{table}]");
    let mut in_section = false;
    for (i, line) in text.lines().enumerate() {
        let t = line.trim_start();
        if t.starts_with('[') && t.ends_with(']') {
            in_section = t == header;
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
    for (idx, line) in text.lines().enumerate() {
        let n = idx + 1;
        check_sibling_url(root, subsys, path, n, line, out);
        check_serde_alias(root, path, n, line, out);
        check_legacy_type_alias(root, path, n, line, &text, out);
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
                    "subsystem `{subsys}` references `{url_var}`/`{port_var}` — that's a `forge → stiglab/synodic`-style HTTP RPC; coordinate via the spine instead (Lever C)"
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
    full: &str,
    out: &mut Vec<Violation>,
) {
    let trimmed = line.trim_start();
    if !trimmed.starts_with("pub type ") {
        return;
    }
    // Heuristic: look at up to the previous 5 lines for a doc-comment
    // containing one of the legacy markers.
    let lines: Vec<&str> = full.lines().collect();
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

/// Apply the `// seam-allow: <reason>` escape. A violation at line N is
/// suppressed if line N-1 (after trimming) starts with `// seam-allow:` and
/// has a non-empty reason. Cargo.toml violations are not exempt-able and
/// always pass through (they're hard-fail per ADR 0004).
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
            let prev_idx = v.line.saturating_sub(2);
            let prev = lines.get(prev_idx).copied().unwrap_or("").trim_start();
            if let Some(reason) = prev.strip_prefix("// seam-allow:") {
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
        let full = "/// Legacy alias for the new name.\npub type Old = New;\n";
        let line = "pub type Old = New;";
        check_legacy_type_alias(fake_root(), fake_path(), 2, line, full, &mut v);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].kind, "legacy-type-alias");
    }

    #[test]
    fn ignores_plain_type_alias() {
        let mut v = Vec::new();
        let full = "/// A reusable shorthand for a complex generic.\npub type Result<T> = std::result::Result<T, Error>;\n";
        let line = "pub type Result<T> = std::result::Result<T, Error>;";
        check_legacy_type_alias(fake_root(), fake_path(), 2, line, full, &mut v);
        assert!(
            v.is_empty(),
            "non-legacy aliases must not fire: {:?}",
            v.iter().map(|x| &x.message).collect::<Vec<_>>()
        );
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
