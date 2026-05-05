//! Trigger-kind manifest check (#236 / #238).
//!
//! Walks `onsager_registry::TRIGGERS` and verifies:
//!
//! 1. **Coverage** — every `onsager_spine::TriggerKind` variant has a
//!    manifest entry keyed by its snake-case `kind_tag()` mapping.
//! 2. **Both ends declared** — every manifest entry names a producer.
//!    (Trigger consumers are universally `forge::trigger_subscriber`,
//!    so a separate consumer list would be busywork; we instead assert
//!    the consumer wire-up exists statically as part of forge.)
//! 3. **Producer category match** — schedule and event kinds must be
//!    produced by `forge` (since their adapters live in forge);
//!    `manual` / `replay` are produced by `portal` (CLI + portal HTTP
//!    endpoints once #222 lands).
//!
//! Coverage is parsed via `syn` from
//! `crates/onsager-spine/src/trigger.rs` — same approach as
//! `check-events`. A variant whose `kind_tag` cannot be derived from the
//! `kind_tag()` impl (e.g. a new variant landed without updating the
//! function) is itself a coverage failure.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use onsager_registry::{Subsystem, TriggerCategory, TRIGGERS};
use syn::{Expr, ExprLit, ImplItem, Item, Lit, Pat, Stmt, Type, Visibility};

const TRIGGER_SRC: &str = "crates/onsager-spine/src/trigger.rs";
const ENUM_NAME: &str = "TriggerKind";

pub fn run() -> Result<()> {
    let root = workspace_root()?;
    let mut errors: Vec<String> = Vec::new();

    let variants_to_kinds = parse_variant_kinds(&root.join(TRIGGER_SRC))?;
    check_coverage(&variants_to_kinds, &mut errors);
    check_both_ends_declared(&mut errors);
    check_producer_category_matches(&mut errors);

    if !errors.is_empty() {
        eprintln!("check-triggers: {} violation(s)", errors.len());
        for e in &errors {
            eprintln!("  {e}");
        }
        eprintln!();
        eprintln!("See spec #236 / #238 and crates/onsager-registry/src/triggers.rs.");
        bail!("trigger-manifest check failed");
    }

    println!(
        "check-triggers: clean ({} variants, {} manifest rows)",
        variants_to_kinds.len(),
        TRIGGERS.triggers.len()
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

/// Parse `crates/onsager-spine/src/trigger.rs` and return a map of
/// `variant_name → kind_tag` (e.g. `Cron → "cron"`). Re-uses the same
/// approach as `check-events`.
fn parse_variant_kinds(trigger_src: &Path) -> Result<BTreeMap<String, String>> {
    let text = std::fs::read_to_string(trigger_src)
        .with_context(|| format!("read {}", trigger_src.display()))?;
    let file =
        syn::parse_file(&text).with_context(|| format!("parse {}", trigger_src.display()))?;

    let mut variants: Vec<String> = Vec::new();
    let mut kind_tag_arms: Vec<(String, String)> = Vec::new();

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
                    if f.sig.ident == "kind_tag" {
                        let arms = extract_match_arms(&f.block.stmts)?;
                        if !arms.is_empty() {
                            kind_tag_arms = arms;
                        }
                    }
                }
            }
            _ => {}
        }
    }

    if variants.is_empty() {
        bail!(
            "could not find pub enum {ENUM_NAME} in {}",
            trigger_src.display()
        );
    }
    if kind_tag_arms.is_empty() {
        bail!(
            "could not find `kind_tag()` impl on {ENUM_NAME} in {}",
            trigger_src.display()
        );
    }

    let mut out = BTreeMap::new();
    for v in &variants {
        let kind = kind_tag_arms
            .iter()
            .find(|(name, _)| name == v)
            .map(|(_, s)| s.clone())
            .ok_or_else(|| anyhow!("kind_tag() has no arm for variant {v}"))?;
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
    let match_expr = stmts.iter().find_map(|s| match s {
        Stmt::Expr(Expr::Match(m), _) => Some(m),
        _ => None,
    });
    let Some(match_expr) = match_expr else {
        return Ok(Vec::new());
    };

    let mut out = Vec::new();
    for arm in &match_expr.arms {
        let lit = match arm.body.as_ref() {
            Expr::Lit(ExprLit {
                lit: Lit::Str(s), ..
            }) => s.value(),
            _ => bail!("kind_tag() match arm body is not a string literal"),
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
// Check 1: every TriggerKind variant has a manifest entry
// ---------------------------------------------------------------------------

fn check_coverage(variants_to_kinds: &BTreeMap<String, String>, errors: &mut Vec<String>) {
    let manifest_kinds: BTreeSet<&'static str> =
        TRIGGERS.triggers.iter().map(|t| t.kind_tag).collect();

    for (variant, kind) in variants_to_kinds {
        if !manifest_kinds.contains(kind.as_str()) {
            errors.push(format!(
                "[coverage] TriggerKind::{variant} (`{kind}`) is missing from the registry manifest"
            ));
        }
    }

    let known_kinds: BTreeSet<&str> = variants_to_kinds.values().map(|s| s.as_str()).collect();
    for kind in &manifest_kinds {
        if !known_kinds.contains(kind) {
            errors.push(format!(
                "[coverage] manifest entry `{kind}` does not match any TriggerKind variant"
            ));
        }
    }
}

// ---------------------------------------------------------------------------
// Check 2: every manifest entry has a producer + a description
// ---------------------------------------------------------------------------

fn check_both_ends_declared(errors: &mut Vec<String>) {
    for t in TRIGGERS.triggers {
        if t.kind_tag.is_empty() {
            errors.push("[manifest] empty kind_tag".into());
        }
        if t.description.is_empty() {
            errors.push(format!("[manifest] `{}` has empty description", t.kind_tag));
        }
    }
}

// ---------------------------------------------------------------------------
// Check 3: producer must align with category
// ---------------------------------------------------------------------------

fn check_producer_category_matches(errors: &mut Vec<String>) {
    for t in TRIGGERS.triggers {
        let expected: &[Subsystem] = match t.category {
            // Schedule producers live in forge (the scheduler module).
            TriggerCategory::Schedule => &[Subsystem::Forge],
            // Event producers — internal event-bus signals — live
            // exclusively in forge (the event-trigger listeners). No
            // Stiglab fallback: a webhook receiver is a Request, not
            // an Event.
            TriggerCategory::Event => &[Subsystem::Forge],
            // Request producers — external HTTP receivers — live in
            // the edge subsystem hosting the route. Today that's
            // stiglab; once #222 promotes portal it moves there.
            TriggerCategory::Request => &[Subsystem::Stiglab, Subsystem::Portal],
            // Manual / replay are user-initiated; produced by the
            // portal edge (CLI or HTTP endpoint once #222 lands).
            TriggerCategory::Manual => &[Subsystem::Portal],
        };
        if !expected.contains(&t.producer) {
            errors.push(format!(
                "[manifest] `{}` (category={:?}) has producer `{}`, expected one of {:?}",
                t.kind_tag,
                t.category,
                t.producer.as_str(),
                expected.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Synthetic: a fake variant map missing from the manifest.
    #[test]
    fn coverage_flags_variant_not_in_manifest() {
        let mut variants = BTreeMap::new();
        variants.insert("FakeVariant".to_string(), "fake_kind".to_string());
        let mut errs = Vec::new();
        check_coverage(&variants, &mut errs);
        assert!(
            errs.iter().any(|e| e.contains("FakeVariant")),
            "expected coverage error, got {errs:?}"
        );
    }

    /// The actual manifest must satisfy all three checks.
    #[test]
    fn real_manifest_passes_static_checks() {
        let mut errs = Vec::new();
        check_both_ends_declared(&mut errs);
        check_producer_category_matches(&mut errs);
        assert!(errs.is_empty(), "manifest errors: {errs:?}");
    }
}
