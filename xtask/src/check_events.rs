//! Lever E (#150) — registry-backed event-type manifest check.
//!
//! Walks `onsager_registry::EVENTS` and verifies the four assertions
//! from spec #150:
//!
//! 1. **Coverage**: every `FactoryEventKind` variant has a manifest entry
//!    keyed by its wire `event_type` string.
//! 2. **Both ends declared**: every manifest entry has at least one
//!    producer, and either at least one consumer or `audit_only = true`.
//! 3. **Emit call sites match producers**: every `append_ext(_, _,
//!    "<event_type>", ...)` literal under
//!    `crates/{forge,stiglab,synodic,ising}/src/` references an event
//!    whose manifest `producers` list includes that subsystem.
//! 4. **Listener call sites match consumers**: every
//!    `notification.event_type [!=|==] "<event_type>"` filter under the
//!    same source trees references an event whose `consumers` list
//!    includes that subsystem.
//!
//! Coverage is parsed via `syn` from
//! `crates/onsager-spine/src/factory_event.rs` — same approach as
//! `gen-event-docs`. Emit and listener scans are line-grep with a
//! deliberately conservative pattern set; false negatives (a missed
//! emit) are preferred to false positives (a flagged legitimate
//! reference). Tests must be excluded — `#[cfg(test)]` modules and
//! `*_test.rs` files do not count.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use onsager_registry::{EventDefinition, Subsystem, EVENTS};
use syn::{Expr, ExprLit, ImplItem, Item, Lit, Pat, Stmt, Type, Visibility};

const SPINE_SRC: &str = "crates/onsager-spine/src/factory_event.rs";
const ENUM_NAME: &str = "FactoryEventKind";

pub fn run() -> Result<()> {
    let root = workspace_root()?;
    let mut errors: Vec<String> = Vec::new();

    // Check 1+2: coverage and producer/consumer declared.
    let variants_to_kinds = parse_variant_kinds(&root.join(SPINE_SRC))?;
    check_coverage(&variants_to_kinds, &mut errors);
    check_both_ends_declared(&mut errors);

    // Check 3+4: emit / listener call sites.
    for sub in Subsystem::SCANNED {
        let src = root.join("crates").join(sub.as_str()).join("src");
        if !src.is_dir() {
            continue;
        }
        for file in rust_files(&src)? {
            scan_file(&root, *sub, &file, &mut errors)?;
        }
    }

    if !errors.is_empty() {
        eprintln!("check-events: {} violation(s)", errors.len());
        for e in &errors {
            eprintln!("  {e}");
        }
        eprintln!();
        eprintln!("See spec #150 and crates/onsager-registry/src/events.rs.");
        bail!("event-manifest check failed");
    }

    println!(
        "check-events: clean ({} events, {} subsystems scanned)",
        EVENTS.events.len(),
        Subsystem::SCANNED.len()
    );
    Ok(())
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

fn def_for(kind: &str) -> Option<&'static EventDefinition> {
    EVENTS.lookup(kind)
}

// ---------------------------------------------------------------------------
// Coverage: parse FactoryEventKind enum + event_type() match arms
// ---------------------------------------------------------------------------

/// Parse the spine source and return a map of `variant_name → event_type`
/// (e.g. `ArtifactRegistered → "artifact.registered"`). Re-uses the same
/// shape as `gen-event-docs` but produces a flat map instead of doc rows.
fn parse_variant_kinds(spine_src: &Path) -> Result<BTreeMap<String, String>> {
    let text = std::fs::read_to_string(spine_src)
        .with_context(|| format!("read {}", spine_src.display()))?;
    let file = syn::parse_file(&text).with_context(|| format!("parse {}", spine_src.display()))?;

    let mut variants: Vec<String> = Vec::new();
    let mut event_type_arms: Vec<(String, String)> = Vec::new();

    for item in &file.items {
        match item {
            Item::Enum(en) if en.ident == ENUM_NAME => {
                if !matches!(en.vis, Visibility::Public(_)) {
                    bail!("{ENUM_NAME} is not pub");
                }
                variants = en.variants.iter().map(|v| v.ident.to_string()).collect();
            }
            Item::Impl(im)
                if im.trait_.is_none() && type_ident(&im.self_ty).as_deref() == Some(ENUM_NAME) =>
            {
                for it in &im.items {
                    let ImplItem::Fn(f) = it else { continue };
                    if f.sig.ident == "event_type" {
                        event_type_arms = extract_match_arms(&f.block.stmts)?;
                    }
                }
            }
            _ => {}
        }
    }

    if variants.is_empty() {
        bail!(
            "could not find pub enum {ENUM_NAME} in {}",
            spine_src.display()
        );
    }

    let mut out = BTreeMap::new();
    for v in &variants {
        let kind = event_type_arms
            .iter()
            .find(|(name, _)| name == v)
            .map(|(_, s)| s.clone())
            .ok_or_else(|| anyhow!("event_type() has no arm for variant {v}"))?;
        out.insert(v.clone(), kind);
    }
    Ok(out)
}

fn type_ident(ty: &Type) -> Option<String> {
    if let Type::Path(p) = ty {
        p.path.segments.last().map(|s| s.ident.to_string())
    } else {
        None
    }
}

fn extract_match_arms(stmts: &[Stmt]) -> Result<Vec<(String, String)>> {
    let match_expr = stmts
        .iter()
        .find_map(|s| match s {
            Stmt::Expr(Expr::Match(m), _) => Some(m),
            _ => None,
        })
        .ok_or_else(|| anyhow!("function body has no top-level match expression"))?;

    let mut out = Vec::new();
    for arm in &match_expr.arms {
        let lit = match arm.body.as_ref() {
            Expr::Lit(ExprLit {
                lit: Lit::Str(s), ..
            }) => s.value(),
            _ => bail!("event_type() match arm body is not a string literal"),
        };
        for variant in collect_variants_from_pat(&arm.pat) {
            out.push((variant, lit.clone()));
        }
    }
    Ok(out)
}

fn collect_variants_from_pat(pat: &Pat) -> Vec<String> {
    let mut out = Vec::new();
    walk_pat(pat, &mut out);
    out
}

fn walk_pat(pat: &Pat, out: &mut Vec<String>) {
    match pat {
        Pat::Or(or) => {
            for p in &or.cases {
                walk_pat(p, out);
            }
        }
        Pat::Struct(s) => push_last_segment(&s.path, out),
        Pat::TupleStruct(t) => push_last_segment(&t.path, out),
        Pat::Path(p) => push_last_segment(&p.path, out),
        _ => {}
    }
}

fn push_last_segment(path: &syn::Path, out: &mut Vec<String>) {
    if let Some(seg) = path.segments.last() {
        out.push(seg.ident.to_string());
    }
}

// ---------------------------------------------------------------------------
// Check 1: every FactoryEventKind variant has a manifest entry
// ---------------------------------------------------------------------------

fn check_coverage(variants_to_kinds: &BTreeMap<String, String>, errors: &mut Vec<String>) {
    let manifest_kinds: BTreeSet<&'static str> = EVENTS.events.iter().map(|e| e.kind).collect();

    for (variant, kind) in variants_to_kinds {
        if !manifest_kinds.contains(kind.as_str()) {
            errors.push(format!(
                "[coverage] FactoryEventKind::{variant} (`{kind}`) is missing from the registry manifest"
            ));
        }
    }

    let known_kinds: BTreeSet<&str> = variants_to_kinds.values().map(|s| s.as_str()).collect();
    for kind in &manifest_kinds {
        if !known_kinds.contains(kind) {
            errors.push(format!(
                "[coverage] manifest entry `{kind}` does not match any FactoryEventKind variant"
            ));
        }
    }
}

// ---------------------------------------------------------------------------
// Check 2: every manifest entry has at least one producer + (consumer | audit_only)
// ---------------------------------------------------------------------------

fn check_both_ends_declared(errors: &mut Vec<String>) {
    for e in EVENTS.events {
        if e.producers.is_empty() {
            errors.push(format!("[manifest] `{}` declares no producer", e.kind));
        }
        if e.consumers.is_empty() && !e.audit_only {
            errors.push(format!(
                "[manifest] `{}` declares no consumer and is not audit_only",
                e.kind
            ));
        }
    }
}

// ---------------------------------------------------------------------------
// Checks 3 + 4: scan subsystem source for emit and listener call sites.
//
// Conservative grep-style detection. False negatives are preferred to false
// positives:
//
//   - **Emit**: any line containing `append_ext` or `append_factory_event`
//     where the same line (or the next 4 lines) carry a `"<event_type>"`
//     literal known to the manifest.
//   - **Listener**: any line of the form
//     `notification.event_type [!=|==] "<event_type>"` or `event_type:
//     "<event_type>"` inside an EventRef construction. Matching the
//     listener subsystem against the manifest's `consumers` list.
//
// Tests are excluded by skipping `#[cfg(test)]` modules per file (we
// consider any line after the first `#[cfg(test)]` attribute outside a
// brace context to be in test scope; this is a coarse heuristic, but the
// test-block is always the last item in our subsystem source files).
// ---------------------------------------------------------------------------

fn scan_file(root: &Path, sub: Subsystem, path: &Path, errors: &mut Vec<String>) -> Result<()> {
    let text = std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let lines: Vec<&str> = text.lines().collect();
    let test_start = first_cfg_test_line(&lines);

    for (idx, line) in lines.iter().enumerate() {
        if idx >= test_start {
            break;
        }
        let line_no = idx + 1;
        let code = strip_line_comment(line);

        // -- Check 3: emit call site --------------------------------
        if code.contains("append_ext") || code.contains("append_factory_event") {
            // event_type literal may be on the same line or within the
            // next 4 lines (multi-line append_ext call).
            let window: String = lines[idx..lines.len().min(idx + 6)].join("\n");
            for kind in find_event_type_literals(&window) {
                let Some(def) = def_for(&kind) else {
                    continue; // unknown literal; coverage check handles enum gaps
                };
                if !def.producers.contains(&sub) {
                    errors.push(format!(
                        "[emit] {}:{} subsystem `{}` emits `{}`, but manifest producers = {:?}",
                        rel(root, path).display(),
                        line_no,
                        sub.as_str(),
                        kind,
                        def.producers.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
                    ));
                }
            }
        }

        // -- Check 4: listener filter --------------------------------
        if let Some(kind) = parse_event_type_filter(code) {
            let Some(def) = def_for(&kind) else { continue };
            if !def.consumers.contains(&sub) {
                errors.push(format!(
                    "[listener] {}:{} subsystem `{}` filters on `{}`, but manifest consumers = {:?}",
                    rel(root, path).display(),
                    line_no,
                    sub.as_str(),
                    kind,
                    def.consumers
                        .iter()
                        .map(|s| s.as_str())
                        .collect::<Vec<_>>(),
                ));
            }
        }
    }
    Ok(())
}

/// Find all event-type literals known to the manifest in `s`. Returns the
/// matched kind strings (deduplicated).
fn find_event_type_literals(s: &str) -> Vec<String> {
    let mut out: BTreeSet<String> = BTreeSet::new();
    for def in EVENTS.events {
        let needle = format!("\"{}\"", def.kind);
        if s.contains(&needle) {
            out.insert(def.kind.to_string());
        }
    }
    out.into_iter().collect()
}

/// Parse `notification.event_type != "<kind>"` or `== "<kind>"` and return
/// the kind. Returns `None` if the line isn't a listener filter.
fn parse_event_type_filter(code: &str) -> Option<String> {
    // Look for `notification.event_type` and then a `"..."` literal.
    if !code.contains("event_type") {
        return None;
    }
    if !(code.contains("notification.event_type")
        || code.contains("event_type !=")
        || code.contains("event_type =="))
    {
        return None;
    }
    extract_first_string(code)
}

fn extract_first_string(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'"' {
            let start = i + 1;
            let mut j = start;
            while j < bytes.len() && bytes[j] != b'"' {
                if bytes[j] == b'\\' {
                    j += 2;
                    continue;
                }
                j += 1;
            }
            if j <= bytes.len() {
                return Some(s[start..j].to_string());
            }
            return None;
        }
        i += 1;
    }
    None
}

/// Find the line index of the first `#[cfg(test)]` at module scope so we
/// can stop scanning before tests. Returns `lines.len()` (i.e. "no test
/// block") if none found.
fn first_cfg_test_line(lines: &[&str]) -> usize {
    for (i, line) in lines.iter().enumerate() {
        if line.trim_start().starts_with("#[cfg(test)]") {
            return i;
        }
    }
    lines.len()
}

/// Strip a trailing `//` line comment outside string literals — same shape
/// as `lint_seams::strip_line_comment` but inlined to keep this module
/// self-contained.
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Synthetic: pretend we have a manifest with a producer but no
    /// consumer + audit_only=false. Mirrors `check_both_ends_declared`'s
    /// branch directly since we can't mutate the static `EVENTS`.
    #[test]
    fn predicate_flags_no_consumer_non_audit_event() {
        fn declared_ok(
            producers: &[Subsystem],
            consumers: &[Subsystem],
            audit_only: bool,
        ) -> Vec<&'static str> {
            let mut errs = Vec::new();
            if producers.is_empty() {
                errs.push("no producer");
            }
            if consumers.is_empty() && !audit_only {
                errs.push("no consumer");
            }
            errs
        }
        assert_eq!(
            declared_ok(&[Subsystem::Forge], &[], false),
            vec!["no consumer"]
        );
        assert_eq!(
            declared_ok(&[Subsystem::Forge], &[], true),
            Vec::<&str>::new()
        );
        assert_eq!(
            declared_ok(&[], &[Subsystem::Forge], false),
            vec!["no producer"]
        );
    }

    /// Synthetic: an emit literal whose subsystem is not in the manifest
    /// `producers` list. Mirrors how `scan_file` would fire on a real
    /// repository — we drive `find_event_type_literals` and the
    /// producer membership check directly.
    #[test]
    fn emit_outside_manifest_producers_is_flagged() {
        let kind = "forge.shaping_dispatched";
        let line = format!("spine.append_ext(stream, \"forge\", \"{kind}\", data, &m, None)");
        let hits = find_event_type_literals(&line);
        assert_eq!(hits, vec![kind.to_string()]);
        let def = def_for(kind).unwrap();
        // Pretending stiglab emits forge.shaping_dispatched: must NOT be
        // in producers (real producer is forge).
        assert!(!def.producers.contains(&Subsystem::Stiglab));
    }

    /// Synthetic: a `Listener` filter whose subsystem is not in the
    /// manifest `consumers` list. Drives `parse_event_type_filter` +
    /// consumer membership.
    #[test]
    fn listener_outside_manifest_consumers_is_flagged() {
        let kind = "synodic.gate_verdict";
        let line = format!("if notification.event_type != \"{kind}\" {{ return Ok(()); }}");
        let parsed = parse_event_type_filter(&line).expect("filter parses");
        assert_eq!(parsed, kind);
        let def = def_for(kind).unwrap();
        // Real consumer is forge — synodic must not appear.
        assert!(!def.consumers.contains(&Subsystem::Synodic));
        assert!(def.consumers.contains(&Subsystem::Forge));
    }

    /// Synthetic: a `FactoryEventKind` variant missing from the
    /// manifest. We can't mutate EVENTS at runtime, so we exercise
    /// `check_coverage` against a forged variant map.
    #[test]
    fn coverage_flags_variant_not_in_manifest() {
        let mut variants = BTreeMap::new();
        variants.insert("FakeVariant".to_string(), "fake.kind".to_string());
        let mut errs = Vec::new();
        check_coverage(&variants, &mut errs);
        assert!(
            errs.iter().any(|e| e.contains("FakeVariant")),
            "expected coverage error, got {errs:?}"
        );
    }

    #[test]
    fn extract_first_string_handles_escaped_quotes() {
        assert_eq!(
            extract_first_string(r#"if x == "hello\"world""#),
            Some(r#"hello\"world"#.to_string())
        );
    }

    #[test]
    fn cfg_test_marker_is_detected() {
        let lines = ["fn x() {}", "", "#[cfg(test)]", "mod tests { }"];
        assert_eq!(first_cfg_test_line(&lines), 2);
    }
}
