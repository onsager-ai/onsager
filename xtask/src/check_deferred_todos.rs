//! Deferred-TODO lint (spec #275) — Occam's-razor projection of "no
//! dangling wires" / "untracked defer".
//!
//! Every TODO-shaped marker in the workspace must point at a tracked
//! issue. The set of markers we care about:
//!
//! - `TODO` (case-insensitive, word-bounded)
//! - `FIXME` (case-insensitive, word-bounded)
//! - `#[allow(dead_code)]`
//! - `// phase \d` (e.g. `// Phase 2`)
//! - `// v\d+\.\d+` (e.g. `// v0.3`)
//!
//! Each occurrence on a non-test line must satisfy one of:
//!
//! - Reference a GitHub issue number `#NNN` on the same line (any
//!   `#NNN` reference counts — open/closed is not checked here, since
//!   that requires network access; closed-link discipline is a review-
//!   time concern).
//! - Carry `// occam-allow: <reason>` on the same line.
//!
//! "Non-test" means files under `crates/*/src/`, `xtask/src/`,
//! `crates/*/tests/`, etc. We exclude generated / vendored content
//! (`target/`, `node_modules/`).
//!
//! ## Modes
//!
//! - `--mode=warn` (default): print violations, exit 0.
//! - `--mode=fail`: exit non-zero on any unallowed violation.
//!
//! Landing in warn mode per spec #275; ratchet to fail in a follow-up.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Warn,
    Fail,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct DeferredHit {
    pub marker: &'static str,
    pub line_no: usize,
    pub text: String,
}

pub fn run(args: Vec<String>) -> Result<()> {
    let mode = parse_mode(&args)?;
    let root = workspace_root()?;

    let files = collect_files(&root)?;
    let mut violations: Vec<(PathBuf, DeferredHit)> = Vec::new();
    let mut allowed: Vec<(PathBuf, DeferredHit, String)> = Vec::new();

    for file in &files {
        if is_self_excluded(&root, file) {
            continue;
        }
        let text =
            std::fs::read_to_string(file).with_context(|| format!("read {}", file.display()))?;
        for hit in scan_text(&text) {
            if let Some(reason) = parse_allow(&hit.text) {
                allowed.push((file.clone(), hit, reason));
                continue;
            }
            if has_issue_ref(&hit.text) {
                continue;
            }
            violations.push((file.clone(), hit));
        }
    }

    if !allowed.is_empty() {
        eprintln!("deferred-TODO occam-allow exemptions:");
        for (file, hit, reason) in &allowed {
            eprintln!(
                "  {}:{} [{}] — allowed: {reason}",
                rel(&root, file).display(),
                hit.line_no,
                hit.marker
            );
        }
        eprintln!();
    }

    if violations.is_empty() {
        println!(
            "check-deferred-todos: clean ({} file(s) scanned, {} allowed exemption(s))",
            files.len(),
            allowed.len()
        );
        return Ok(());
    }

    eprintln!(
        "check-deferred-todos: {} violation(s) — markers without an issue link:",
        violations.len()
    );
    for (file, hit) in &violations {
        eprintln!(
            "  {}:{} [{}] {}",
            rel(&root, file).display(),
            hit.line_no,
            hit.marker,
            hit.text.trim()
        );
    }
    eprintln!();
    eprintln!("See spec #275 (Occam's-razor lints). Each marker must either");
    eprintln!("reference a GitHub issue (`#NNN`) on the same line, or carry");
    eprintln!("`// occam-allow: <reason>` on the same line.");

    match mode {
        Mode::Warn => {
            eprintln!("(warn mode — not failing)");
            Ok(())
        }
        Mode::Fail => bail!("deferred-TODO lint failed"),
    }
}

fn parse_mode(args: &[String]) -> Result<Mode> {
    let mut mode = Mode::Warn;
    for arg in args {
        match arg.as_str() {
            "--mode=warn" => mode = Mode::Warn,
            "--mode=fail" => mode = Mode::Fail,
            other => bail!("unknown flag for check-deferred-todos: {other}"),
        }
    }
    Ok(mode)
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

fn collect_files(root: &Path) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    for sub in ["crates", "xtask", "apps/dashboard/src"] {
        walk(&root.join(sub), &mut out)?;
    }
    out.sort();
    Ok(out)
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    if !dir.is_dir() {
        return Ok(());
    }
    for entry in std::fs::read_dir(dir).with_context(|| format!("read_dir {}", dir.display()))? {
        let entry = entry?;
        let path = entry.path();
        let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
        if path.is_dir() {
            if matches!(
                name,
                "target" | "node_modules" | "dist" | "build" | ".next" | ".turbo" | "out"
            ) {
                continue;
            }
            walk(&path, out)?;
        } else {
            let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("");
            if matches!(ext, "rs" | "ts" | "tsx" | "js" | "jsx") {
                out.push(path);
            }
        }
    }
    Ok(())
}

pub(crate) fn scan_text(text: &str) -> Vec<DeferredHit> {
    let mut out = Vec::new();
    let stripped = strip_string_literals(text);
    let original: Vec<&str> = text.lines().collect();
    for (idx, line) in stripped.lines().enumerate() {
        let line_no = idx + 1;
        for marker in find_markers(line) {
            let original_line = original.get(idx).copied().unwrap_or(line);
            out.push(DeferredHit {
                marker,
                line_no,
                text: original_line.to_string(),
            });
        }
    }
    out
}

/// Lint files own this module's source by design (they enumerate the
/// markers). Excluding it is cleaner than blanketing the file in
/// `// occam-allow:` comments. The xtask/tests/fixtures directory
/// hosts deliberate violations for unit tests — same rationale.
fn is_self_excluded(root: &Path, path: &Path) -> bool {
    let p = path.strip_prefix(root).unwrap_or(path);
    let s = p.to_string_lossy();
    s == "xtask/src/check_deferred_todos.rs" || s.starts_with("xtask/tests/fixtures/")
}

/// Replace contents of Rust / TS string literals with spaces so marker
/// patterns inside template prompts ("flag unrelated TODO additions")
/// don't trip the lint. Preserves line structure (newlines pass
/// through) so reported line numbers still match the original file.
///
/// Handles three literal kinds (best-effort, line-oriented):
/// - `"..."` — single-line, backslash-escape aware.
/// - `r"..."` / `r#"..."#` — raw, possibly multi-line, hash-balanced.
/// - line comments are not stripped — that's where TODOs live.
fn strip_string_literals(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if c == b'/' && bytes.get(i + 1) == Some(&b'/') {
            // Line comment — keep verbatim until end of line.
            while i < bytes.len() && bytes[i] != b'\n' {
                out.push(bytes[i] as char);
                i += 1;
            }
            continue;
        }
        if c == b'r' {
            // Possible raw string: r#*"...".
            let mut j = i + 1;
            let mut hashes = 0;
            while j < bytes.len() && bytes[j] == b'#' {
                hashes += 1;
                j += 1;
            }
            if j < bytes.len() && bytes[j] == b'"' {
                out.push('r');
                for _ in 0..hashes {
                    out.push('#');
                }
                out.push('"');
                j += 1;
                while j < bytes.len() {
                    if bytes[j] == b'"' {
                        let mut k = j + 1;
                        let mut close_hashes = 0;
                        while k < bytes.len() && bytes[k] == b'#' && close_hashes < hashes {
                            close_hashes += 1;
                            k += 1;
                        }
                        if close_hashes == hashes {
                            out.push('"');
                            for _ in 0..hashes {
                                out.push('#');
                            }
                            j = k;
                            break;
                        }
                    }
                    if bytes[j] == b'\n' {
                        out.push('\n');
                    } else {
                        out.push(' ');
                    }
                    j += 1;
                }
                i = j;
                continue;
            }
        }
        if c == b'"' {
            out.push('"');
            i += 1;
            while i < bytes.len() {
                if bytes[i] == b'\\' && i + 1 < bytes.len() {
                    // Escape pair: emit space for `\` and preserve a literal
                    // newline if the escape is a Rust line-continuation
                    // (`\` + newline). Otherwise emit a space for the
                    // escaped character so line counts stay aligned.
                    out.push(' ');
                    if bytes[i + 1] == b'\n' {
                        out.push('\n');
                    } else {
                        out.push(' ');
                    }
                    i += 2;
                    continue;
                }
                if bytes[i] == b'"' {
                    out.push('"');
                    i += 1;
                    break;
                }
                if bytes[i] == b'\n' {
                    // Unterminated string — flush newline and break.
                    out.push('\n');
                    i += 1;
                    break;
                }
                out.push(' ');
                i += 1;
            }
            continue;
        }
        out.push(c as char);
        i += 1;
    }
    out
}

fn find_markers(line: &str) -> Vec<&'static str> {
    let mut out: Vec<&'static str> = Vec::new();
    // Skip the marker hits inside the lint module's own pattern list —
    // this file documents the markers in `//!` docs and would self-trip
    // otherwise. The escape: an explicit "occam-allow: lint self-ref"
    // on the same line. We don't need a special case here.
    if has_word(line, "TODO") {
        out.push("TODO");
    }
    if has_word(line, "FIXME") {
        out.push("FIXME");
    }
    if line.contains("#[allow(dead_code)]") {
        out.push("dead_code");
    }
    if matches_phase(line) {
        out.push("phase");
    }
    if matches_version(line) {
        out.push("version");
    }
    out.dedup();
    out
}

fn has_word(line: &str, word: &str) -> bool {
    let upper = line.to_uppercase();
    let mut idx = 0;
    while let Some(pos) = upper[idx..].find(word) {
        let start = idx + pos;
        let end = start + word.len();
        let before_ok = start == 0 || !is_ident_char(upper.as_bytes()[start - 1]);
        let after_ok = end == upper.len() || !is_ident_char(upper.as_bytes()[end]);
        if before_ok && after_ok {
            return true;
        }
        idx = end;
    }
    false
}

fn is_ident_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// Match `// phase N` (case-insensitive). The `//` prefix anchors this
/// to comment lines, so the word "phase" in prose docs doesn't trip.
fn matches_phase(line: &str) -> bool {
    let lower = line.to_lowercase();
    for (idx, _) in lower.match_indices("phase ") {
        // Require a `//` somewhere to the left of the match — comment-anchored.
        if !lower[..idx].contains("//") {
            continue;
        }
        let after = &lower[idx + "phase ".len()..];
        if after.chars().next().is_some_and(|c| c.is_ascii_digit()) {
            return true;
        }
    }
    false
}

/// Match `// vN.M`, comment-anchored. The same `//` constraint avoids
/// crate doc-comments referencing version numbers like "v0.2" from
/// tripping.
fn matches_version(line: &str) -> bool {
    let lower = line.to_lowercase();
    for (idx, _) in lower.match_indices('v') {
        if idx == 0 {
            continue;
        }
        let prev = lower.as_bytes()[idx - 1];
        if is_ident_char(prev) {
            continue;
        }
        if !lower[..idx].contains("//") {
            continue;
        }
        let rest = &lower[idx + 1..];
        let mut chars = rest.chars();
        let Some(c1) = chars.next() else { continue };
        if !c1.is_ascii_digit() {
            continue;
        }
        // Need at least one '.' followed by a digit before whitespace /
        // word-end.
        let after_first = &rest[c1.len_utf8()..];
        let dot_idx = after_first.find('.');
        if let Some(d) = dot_idx
            && after_first[d + 1..]
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_digit())
        {
            return true;
        }
    }
    false
}

pub(crate) fn has_issue_ref(line: &str) -> bool {
    // Look for `#` followed by 1+ ASCII digits, not part of a fragment
    // or anchor inside a Markdown link header (`# Heading`).
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'#' {
            let start = i + 1;
            let mut j = start;
            while j < bytes.len() && bytes[j].is_ascii_digit() {
                j += 1;
            }
            if j > start {
                return true;
            }
        }
        i += 1;
    }
    false
}

fn parse_allow(line: &str) -> Option<String> {
    let idx = line.find("// occam-allow:")?;
    let reason = line[idx + "// occam-allow:".len()..].trim();
    if reason.is_empty() {
        None
    } else {
        Some(reason.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn todo_without_issue_is_flagged() {
        let hits = scan_text("// TODO: clean this up\nfn x() {}\n");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].marker, "TODO");
        assert!(!has_issue_ref(&hits[0].text));
    }

    #[test]
    fn todo_with_issue_is_passed() {
        let hits = scan_text("// TODO(#275): land the lints\n");
        assert_eq!(hits.len(), 1);
        assert!(has_issue_ref(&hits[0].text));
    }

    #[test]
    fn fixme_is_a_marker() {
        let hits = scan_text("// FIXME: broken on edge case\n");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].marker, "FIXME");
    }

    #[test]
    fn dead_code_attr_is_a_marker() {
        let hits = scan_text("#[allow(dead_code)]\nfn unused() {}\n");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].marker, "dead_code");
    }

    #[test]
    fn phase_pattern_is_a_marker() {
        let hits = scan_text("// phase 2: wire up the dashboard\n");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].marker, "phase");
    }

    #[test]
    fn version_pattern_is_a_marker() {
        let hits = scan_text("// v0.3 will need this\n");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].marker, "version");
    }

    #[test]
    fn word_boundary_excludes_subwords() {
        // "todoist" should not match TODO.
        let hits = scan_text("// todoist: not a marker\n");
        assert!(hits.is_empty());
    }

    #[test]
    fn occam_allow_inline_is_recognized() {
        let line = "// TODO: revisit later // occam-allow: external-API quirk";
        assert_eq!(parse_allow(line).as_deref(), Some("external-API quirk"));
    }

    #[test]
    fn issue_ref_detects_hash_number() {
        assert!(has_issue_ref("see #275 for context"));
        assert!(has_issue_ref("#1"));
        assert!(!has_issue_ref("no number here"));
        assert!(!has_issue_ref("# Heading"));
    }

    #[test]
    fn marker_inside_string_literal_is_ignored() {
        // The text inside the prompt would otherwise trip TODO.
        let src = r#"let prompt = "review the diff for TODO additions";
fn x() {}
"#;
        let hits = scan_text(src);
        assert!(
            hits.is_empty(),
            "string-literal TODO must not trip the lint: {hits:?}"
        );
    }

    #[test]
    fn marker_inside_raw_string_is_ignored() {
        let src = "let prompt = r#\"flag TODO/FIXME additions in the diff\"#;\nfn x() {}\n";
        let hits = scan_text(src);
        assert!(
            hits.is_empty(),
            "raw-string content must not trip: {hits:?}"
        );
    }

    #[test]
    fn marker_in_comment_outside_string_is_still_flagged() {
        let src = "let s = \"safe\"; // TODO untracked\nfn x() {}\n";
        let hits = scan_text(src);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].marker, "TODO");
    }
}
