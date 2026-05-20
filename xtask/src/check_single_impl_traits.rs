//! Single-impl-trait lint (spec #275) — Occam's-razor projection of
//! "internal symmetry / no premature abstraction": a `pub trait` with
//! exactly one implementor is a speculative abstraction. Either inline
//! the body or stop calling it a trait until a second implementor
//! arrives.
//!
//! ## What's counted
//!
//! Walks every `.rs` file under the workspace's source trees
//! (`crates/*/src/`, `xtask/src/`), parsing each with `syn`.
//!
//! - Trait defs: any `pub trait` (i.e. visible to other crates) in
//!   non-test code (we **do not** parse traits defined inside
//!   `#[cfg(test)]` modules; a test-only trait is private by
//!   convention).
//! - Impl blocks: every `impl <TraitName> for <Type>` in any `.rs`
//!   file, **including** `#[cfg(test)]` modules. Per the spec's
//!   "Human decides" call: counting test impls toward the count
//!   means a mock-impl-only trait isn't flagged as single-impl —
//!   testability counts.
//!
//! Trait → impl matching is keyed by `(crate, trait_name)`: each `pub
//! trait` is attributed to the crate that owns the file it lives in,
//! and each `impl <Trait> for <Type>` is attributed to its file's crate
//! when `<Trait>` is a bare identifier, or to the leading path segment
//! when the trait is qualified (e.g. `impl crate::Foo for X` stays in
//! the impl's crate; `impl onsager_substrate::Executor for X` is
//! attributed to `onsager-substrate`). This avoids merging
//! same-named-but-distinct traits across crates (e.g. the workspace
//! defines two `pub trait Executor`s).
//!
//! ## Escape hatch
//!
//! `// occam-allow: <reason>` on a line immediately above the
//! `pub trait Foo` declaration exempts it. Mirrors the seam-allow shape.
//!
//! ## Modes
//!
//! - `--mode=warn` (default): print violations, exit 0.
//! - `--mode=fail`: exit non-zero on any unallowed violation.
//!
//! Landing in warn mode per spec #275; ratchet to fail in a follow-up.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use syn::{File as SynFile, Item, ItemImpl, ItemTrait, Visibility};

use crate::check_orphan_crates::parse_occam_allow;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Warn,
    Fail,
}

/// Composite key: `(crate_name, trait_name)`. Two crates each defining
/// `pub trait Executor` produce distinct keys and aren't merged.
type TraitKey = (String, String);

#[derive(Debug)]
struct TraitDef {
    key: TraitKey,
    file: PathBuf,
    line: usize,
    allow: Option<String>,
}

impl TraitDef {
    fn display_name(&self) -> &str {
        &self.key.1
    }
}

pub fn run(args: Vec<String>) -> Result<()> {
    let mode = parse_mode(&args)?;
    let root = workspace_root()?;

    let files = collect_rust_files(&root)?;

    let mut trait_defs: Vec<TraitDef> = Vec::new();
    let mut impl_counts: BTreeMap<TraitKey, usize> = BTreeMap::new();

    for file in &files {
        let text =
            std::fs::read_to_string(file).with_context(|| format!("read {}", file.display()))?;
        let parsed = match syn::parse_file(&text) {
            Ok(f) => f,
            Err(_) => continue, // bad syntax — skip; cargo will fail it elsewhere.
        };
        let crate_name = crate_for(&root, file);
        scan(
            &parsed,
            file,
            &text,
            &crate_name,
            &mut trait_defs,
            &mut impl_counts,
        );
    }

    let mut violations: Vec<&TraitDef> = Vec::new();
    let mut allowed: Vec<(&TraitDef, &str)> = Vec::new();

    for td in &trait_defs {
        let count = impl_counts.get(&td.key).copied().unwrap_or(0);
        if count != 1 {
            continue;
        }
        if let Some(reason) = &td.allow {
            allowed.push((td, reason.as_str()));
        } else {
            violations.push(td);
        }
    }

    if !allowed.is_empty() {
        eprintln!("single-impl-trait occam-allow exemptions:");
        for (td, reason) in &allowed {
            eprintln!(
                "  {}:{} `{}::{}` — allowed: {reason}",
                rel(&root, &td.file).display(),
                td.line,
                td.key.0,
                td.display_name()
            );
        }
        eprintln!();
    }

    if violations.is_empty() {
        println!(
            "check-single-impl-traits: clean ({} pub trait(s) scanned, {} allowed exemption(s))",
            trait_defs.len(),
            allowed.len()
        );
        return Ok(());
    }

    eprintln!(
        "check-single-impl-traits: {} violation(s) — pub traits with exactly one impl:",
        violations.len()
    );
    for td in &violations {
        eprintln!(
            "  {}:{} `{}::{}` — 1 impl in the workspace",
            rel(&root, &td.file).display(),
            td.line,
            td.key.0,
            td.display_name()
        );
    }
    eprintln!();
    eprintln!("See spec #275 (Occam's-razor lints). To exempt, add");
    eprintln!("`// occam-allow: <reason>` on the line above the `pub trait` decl.");

    match mode {
        Mode::Warn => {
            eprintln!("(warn mode — not failing)");
            Ok(())
        }
        Mode::Fail => bail!("single-impl-trait lint failed"),
    }
}

fn parse_mode(args: &[String]) -> Result<Mode> {
    let mut mode = Mode::Warn;
    for arg in args {
        match arg.as_str() {
            "--mode=warn" => mode = Mode::Warn,
            "--mode=fail" => mode = Mode::Fail,
            other => bail!("unknown flag for check-single-impl-traits: {other}"),
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

fn collect_rust_files(root: &Path) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    let crates_dir = root.join("crates");
    walk_rust(&crates_dir, &mut out)?;
    let xtask_dir = root.join("xtask").join("src");
    if xtask_dir.is_dir() {
        walk_rust(&xtask_dir, &mut out)?;
    }
    out.sort();
    Ok(out)
}

fn walk_rust(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    if !dir.is_dir() {
        return Ok(());
    }
    for entry in std::fs::read_dir(dir).with_context(|| format!("read_dir {}", dir.display()))? {
        let entry = entry?;
        let path = entry.path();
        let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
        if path.is_dir() {
            if matches!(name, "target" | "node_modules") {
                continue;
            }
            walk_rust(&path, out)?;
        } else if path.extension().and_then(|s| s.to_str()) == Some("rs") {
            out.push(path);
        }
    }
    Ok(())
}

fn scan(
    file: &SynFile,
    path: &Path,
    text: &str,
    crate_name: &str,
    trait_defs: &mut Vec<TraitDef>,
    impl_counts: &mut BTreeMap<TraitKey, usize>,
) {
    let lines: Vec<&str> = text.lines().collect();
    for item in &file.items {
        scan_item(
            item,
            path,
            &lines,
            crate_name,
            trait_defs,
            impl_counts,
            /*in_test=*/ false,
        );
    }
}

fn scan_item(
    item: &Item,
    path: &Path,
    lines: &[&str],
    crate_name: &str,
    trait_defs: &mut Vec<TraitDef>,
    impl_counts: &mut BTreeMap<TraitKey, usize>,
    in_test: bool,
) {
    match item {
        Item::Trait(t) => {
            if in_test {
                return;
            }
            collect_trait(t, path, lines, crate_name, trait_defs);
        }
        Item::Impl(im) => {
            collect_impl(im, crate_name, impl_counts);
        }
        Item::Mod(m) => {
            let mod_is_test = in_test || has_cfg_test(&m.attrs);
            if let Some((_, items)) = &m.content {
                for inner in items {
                    scan_item(
                        inner,
                        path,
                        lines,
                        crate_name,
                        trait_defs,
                        impl_counts,
                        mod_is_test,
                    );
                }
            }
        }
        _ => {}
    }
}

fn collect_trait(
    t: &ItemTrait,
    path: &Path,
    lines: &[&str],
    crate_name: &str,
    trait_defs: &mut Vec<TraitDef>,
) {
    if !matches!(t.vis, Visibility::Public(_)) {
        return;
    }
    let line = t.ident.span().start().line;
    let allow = if line >= 2 {
        let above = lines.get(line.saturating_sub(2)).copied().unwrap_or("");
        parse_occam_allow(above)
    } else {
        None
    };
    trait_defs.push(TraitDef {
        key: (crate_name.to_string(), t.ident.to_string()),
        file: path.to_path_buf(),
        line,
        allow,
    });
}

fn collect_impl(im: &ItemImpl, crate_name: &str, impl_counts: &mut BTreeMap<TraitKey, usize>) {
    let Some((_, path, _)) = &im.trait_ else {
        return;
    };
    let Some(last_seg) = path.segments.last() else {
        return;
    };
    let trait_name = last_seg.ident.to_string();
    // Qualified path like `onsager_substrate::Executor` → attribute to
    // the leading segment (converted from `onsager_substrate` style
    // back to `onsager-substrate` for matching against the crate name
    // derived from the path). A leading `crate::` / `self::` /
    // `super::` keeps the impl's own crate. A bare identifier
    // (`Executor`) is also local to the impl's crate.
    let owning_crate = if path.segments.len() > 1 {
        let leading = path.segments.first().unwrap().ident.to_string();
        match leading.as_str() {
            "crate" | "self" | "super" => crate_name.to_string(),
            _ => leading.replace('_', "-"),
        }
    } else {
        crate_name.to_string()
    };
    *impl_counts.entry((owning_crate, trait_name)).or_insert(0) += 1;
}

/// Derive the crate name from a path under `crates/<name>/...`. Falls
/// back to `xtask` for paths under `xtask/`, and to "<unknown>" for
/// anything else (so the key still namespaces the trait, just under a
/// shared bucket — the lint stays sound, it just won't merge two
/// outside-tree files into one crate's namespace).
fn crate_for(root: &Path, file: &Path) -> String {
    let rel = file.strip_prefix(root).unwrap_or(file);
    let mut comps = rel.components();
    match comps.next().and_then(|c| c.as_os_str().to_str()) {
        Some("crates") => comps
            .next()
            .and_then(|c| c.as_os_str().to_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| "<unknown>".to_string()),
        Some("xtask") => "xtask".to_string(),
        _ => "<unknown>".to_string(),
    }
}

fn has_cfg_test(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|a| {
        let path = a.path();
        if !path.is_ident("cfg") {
            return false;
        }
        let mut found = false;
        let _ = a.parse_nested_meta(|m| {
            if m.path.is_ident("test") {
                found = true;
            }
            Ok(())
        });
        found
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(src: &str) -> SynFile {
        syn::parse_file(src).expect("parse fixture")
    }

    fn key(crate_name: &str, trait_name: &str) -> TraitKey {
        (crate_name.to_string(), trait_name.to_string())
    }

    #[test]
    fn pub_trait_with_exactly_one_impl_is_flagged() {
        let src = r#"
pub trait Doer { fn do_it(&self); }
struct A;
impl Doer for A { fn do_it(&self) {} }
"#;
        let file = parse(src);
        let mut traits = Vec::new();
        let mut counts = BTreeMap::new();
        scan(
            &file,
            Path::new("/tmp/x.rs"),
            src,
            "demo",
            &mut traits,
            &mut counts,
        );
        assert_eq!(traits.len(), 1);
        assert_eq!(traits[0].key, key("demo", "Doer"));
        assert_eq!(counts.get(&key("demo", "Doer")), Some(&1));
    }

    #[test]
    fn private_trait_is_ignored() {
        let src = r#"
trait Private { fn x(&self); }
struct A;
impl Private for A { fn x(&self) {} }
"#;
        let file = parse(src);
        let mut traits = Vec::new();
        let mut counts = BTreeMap::new();
        scan(
            &file,
            Path::new("/tmp/x.rs"),
            src,
            "demo",
            &mut traits,
            &mut counts,
        );
        assert!(traits.is_empty(), "private traits are not in scope");
    }

    #[test]
    fn test_impl_is_counted_toward_total() {
        let src = r#"
pub trait Doer { fn x(&self); }
struct Real;
impl Doer for Real { fn x(&self) {} }

#[cfg(test)]
mod tests {
    use super::*;
    struct Mock;
    impl Doer for Mock { fn x(&self) {} }
}
"#;
        let file = parse(src);
        let mut traits = Vec::new();
        let mut counts = BTreeMap::new();
        scan(
            &file,
            Path::new("/tmp/x.rs"),
            src,
            "demo",
            &mut traits,
            &mut counts,
        );
        // Mock impl + real impl = 2 → not flagged as single-impl.
        assert_eq!(counts.get(&key("demo", "Doer")), Some(&2));
    }

    #[test]
    fn trait_defined_in_test_module_is_ignored() {
        let src = r#"
#[cfg(test)]
mod tests {
    pub trait TestOnly { fn x(&self); }
    struct A;
    impl TestOnly for A { fn x(&self) {} }
}
"#;
        let file = parse(src);
        let mut traits = Vec::new();
        let mut counts = BTreeMap::new();
        scan(
            &file,
            Path::new("/tmp/x.rs"),
            src,
            "demo",
            &mut traits,
            &mut counts,
        );
        assert!(traits.is_empty(), "test-module-only traits are skipped");
    }

    #[test]
    fn occam_allow_above_trait_is_recognized() {
        let src = "// occam-allow: deliberate seam for a future impl\npub trait Seam { fn x(&self); }\nstruct A;\nimpl Seam for A { fn x(&self) {} }\n";
        let file = parse(src);
        let mut traits = Vec::new();
        let mut counts = BTreeMap::new();
        scan(
            &file,
            Path::new("/tmp/x.rs"),
            src,
            "demo",
            &mut traits,
            &mut counts,
        );
        assert_eq!(traits.len(), 1);
        assert_eq!(
            traits[0].allow.as_deref(),
            Some("deliberate seam for a future impl")
        );
    }

    /// Regression for PR #426 review: two crates each defining
    /// `pub trait Executor` with one impl each must not merge into one
    /// 2-impl bucket. Each crate's trait counts independently and is
    /// flagged as single-impl in its own namespace.
    #[test]
    fn same_named_trait_in_two_crates_does_not_merge() {
        let crate_a = r#"
pub trait Executor { fn run(&self); }
struct ImplA;
impl Executor for ImplA { fn run(&self) {} }
"#;
        let crate_b = r#"
pub trait Executor { fn go(&self); }
struct ImplB;
impl Executor for ImplB { fn go(&self) {} }
"#;
        let mut traits = Vec::new();
        let mut counts = BTreeMap::new();
        scan(
            &parse(crate_a),
            Path::new("/tmp/a.rs"),
            crate_a,
            "crate-a",
            &mut traits,
            &mut counts,
        );
        scan(
            &parse(crate_b),
            Path::new("/tmp/b.rs"),
            crate_b,
            "crate-b",
            &mut traits,
            &mut counts,
        );
        assert_eq!(counts.get(&key("crate-a", "Executor")), Some(&1));
        assert_eq!(counts.get(&key("crate-b", "Executor")), Some(&1));
        assert_eq!(traits.len(), 2);
    }

    /// `impl other_crate::Trait for X` attributes the impl to the
    /// leading path segment, so cross-crate impls increment the right
    /// trait's bucket.
    #[test]
    fn qualified_impl_path_attributes_to_leading_segment() {
        let src = r#"
struct LocalImpl;
impl onsager_substrate::Executor for LocalImpl { fn run(&self) {} }
"#;
        let mut traits = Vec::new();
        let mut counts = BTreeMap::new();
        scan(
            &parse(src),
            Path::new("/tmp/x.rs"),
            src,
            "onsager-nodes",
            &mut traits,
            &mut counts,
        );
        assert_eq!(counts.get(&key("onsager-substrate", "Executor")), Some(&1));
        assert!(!counts.contains_key(&key("onsager-nodes", "Executor")));
    }

    #[test]
    fn crate_for_extracts_name_from_crates_path() {
        let root = Path::new("/repo");
        assert_eq!(
            crate_for(
                root,
                Path::new("/repo/crates/onsager-nodes/src/executor.rs")
            ),
            "onsager-nodes"
        );
        assert_eq!(
            crate_for(root, Path::new("/repo/xtask/src/lib.rs")),
            "xtask"
        );
    }
}
