//! File-budget lint (spec #261, Move 4) — bounds per-file token cost so the
//! inner-loop tax of "open one file" stays small.
//!
//! Counts tokens with `tiktoken-rs` against the `o200k_base` encoding. The
//! BPE merges file is vendored in-tree at `xtask/assets/o200k_base.tiktoken`
//! so CI runs are offline and deterministic — the bytes are pinned in the
//! repo, not implicit in whatever `tiktoken-rs` happens to ship. Counts are
//! within ~10% of Claude's own tokenizer; absolute numbers are stable
//! across machines.
//!
//! ## What's counted
//!
//! - **Rust** (`.rs`): top-level items annotated with `#[cfg(test)]` (and
//!   `#[cfg(test)] mod tests { ... }` blocks nested one level deep) are
//!   stripped before counting. Test bulk doesn't pay the prod-read tax.
//! - **TypeScript** (`.ts`/`.tsx`): files matching `*.test.{ts,tsx}` /
//!   `*.spec.{ts,tsx}`, and any file under a `__tests__/` directory, are
//!   skipped entirely.
//!
//! Path-level test/example/bench dirs (`tests/`, `examples/`, `benches/`,
//! `__tests__/`) and Rust sibling test files (`tests.rs`, `*_tests.rs`)
//! are skipped. Build outputs (`target`, `node_modules`, `dist`, `build`,
//! `.next`, `.turbo`, `out`, `coverage`) are not walked.
//!
//! ## Allow-list
//!
//! `// budget-allow: <non-empty reason>` anywhere in the file exempts it
//! from the budget. Reason text is mandatory and grep-able. Mirrors the
//! shape of `// seam-allow:` in `lint_seams`.
//!
//! ## Modes
//!
//! - `--mode=warn` (default): print every over-budget file and exit 0.
//! - `--mode=fail`: exit non-zero on any over-budget file.
//!
//! Spec #261's warn-first sequencing means this lands in `warn` mode
//! first; the per-Move splits (1a-d, 2, 3) each show their drop in CI
//! logs; Move 4d flips the default to `fail`.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use base64::{engine::general_purpose, Engine as _};
use rustc_hash::FxHashMap;
use syn::spanned::Spanned;
use syn::{Attribute, File as SynFile, Item, Meta};
use tiktoken_rs::{CoreBPE, Rank, ENDOFPROMPT, ENDOFTEXT, O200K_BASE_PAT_STR};

/// BPE merges file for the o200k_base encoding. Pinned in-tree so CI is
/// offline and deterministic regardless of which `tiktoken-rs` ships.
const VENDORED_VOCAB: &str = include_str!("../assets/o200k_base.tiktoken");

/// Initial budget per spec #261 — calibrated so today's six worst offenders
/// fail and reasonable files don't. Ratchet planned to ~5000–6000 in a
/// follow-up after Moves 1–3 land.
const DEFAULT_BUDGET: usize = 8000;

const ALLOW_PREFIX: &str = "// budget-allow:";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Warn,
    Fail,
}

/// Count prod tokens in a single file and print the integer count to
/// stdout. Used by `just measure-tokens` to compare against Anthropic's
/// `count_tokens` API. Applies the same prod-content rules as the lint
/// (strip `#[cfg(test)]` for Rust; refuse to count test-named TS files).
pub fn run_count(args: Vec<String>) -> Result<()> {
    if args.len() != 1 {
        bail!("usage: cargo run -p xtask -- count-tokens <file>");
    }
    let file = PathBuf::from(&args[0]);
    let abs = if file.is_absolute() {
        file.clone()
    } else {
        std::env::current_dir().context("getcwd")?.join(&file)
    };
    let src = std::fs::read_to_string(&abs).with_context(|| format!("read {}", abs.display()))?;
    let prod = match abs.extension().and_then(|s| s.to_str()) {
        Some("rs") => strip_rust_cfg_test(&src).unwrap_or_else(|_| src.clone()),
        _ => src.clone(),
    };
    let bpe = build_bpe()?;
    let tokens = bpe.encode_ordinary(&prod).len();
    println!("{tokens}");
    Ok(())
}

pub fn run(args: Vec<String>) -> Result<()> {
    let (mode, budget, paths) = parse_args(args)?;

    let root = workspace_root()?;
    let bpe = build_bpe()?;

    let bases: Vec<PathBuf> = if paths.is_empty() {
        default_scan_roots(&root)
    } else {
        paths
            .into_iter()
            .map(|p| if p.is_absolute() { p } else { root.join(p) })
            .collect()
    };

    let mut over: Vec<Report> = Vec::new();
    let mut allowed: Vec<Report> = Vec::new();
    let mut total = 0usize;

    for base in &bases {
        if !base.exists() {
            continue;
        }
        for file in walk_source_files(base) {
            let rel = file.strip_prefix(&root).unwrap_or(&file).to_path_buf();
            if !is_prod_source(&rel) {
                continue;
            }
            total += 1;

            let src = std::fs::read_to_string(&file)
                .with_context(|| format!("read {}", file.display()))?;
            let allow = find_allow(&src);
            let prod = match rel.extension().and_then(|s| s.to_str()) {
                Some("rs") => strip_rust_cfg_test(&src).unwrap_or_else(|_| src.clone()),
                _ => src.clone(),
            };
            let tokens = bpe.encode_ordinary(&prod).len();

            if tokens > budget {
                let report = Report {
                    path: rel,
                    tokens,
                    budget,
                    allow,
                };
                if report.allow.is_some() {
                    allowed.push(report);
                } else {
                    over.push(report);
                }
            }
        }
    }

    over.sort_by_key(|r| std::cmp::Reverse(r.tokens));
    allowed.sort_by_key(|r| std::cmp::Reverse(r.tokens));

    if !allowed.is_empty() {
        eprintln!("budget-allow exemptions used:");
        for r in &allowed {
            eprintln!(
                "  {}: {} prod tokens (budget {}) — allowed: {}",
                r.path.display(),
                r.tokens,
                r.budget,
                r.allow.as_deref().unwrap_or("")
            );
        }
        eprintln!();
    }

    if !over.is_empty() {
        let header = match mode {
            Mode::Warn => "file-budget WARN (warn-only mode):",
            Mode::Fail => "file-budget violations:",
        };
        eprintln!("{header}");
        for r in &over {
            eprintln!(
                "  {}: {} prod tokens (over budget of {})",
                r.path.display(),
                r.tokens,
                r.budget
            );
        }
        eprintln!(
            "\nTo fix: split the file, or add `// budget-allow: <non-empty reason>` near the top."
        );
        eprintln!();
    }

    eprintln!(
        "scanned {total} prod files; {} over budget; {} allow-exempted (budget = {budget} prod tokens, encoding = o200k_base)",
        over.len(),
        allowed.len(),
    );

    if mode == Mode::Fail && !over.is_empty() {
        bail!(
            "{} file(s) exceed the {budget}-prod-token budget",
            over.len()
        );
    }
    Ok(())
}

#[derive(Debug)]
struct Report {
    path: PathBuf,
    tokens: usize,
    budget: usize,
    allow: Option<String>,
}

fn parse_args(args: Vec<String>) -> Result<(Mode, usize, Vec<PathBuf>)> {
    let mut mode = Mode::Warn;
    let mut budget: usize = DEFAULT_BUDGET;
    let mut paths: Vec<PathBuf> = Vec::new();
    let mut iter = args.into_iter();
    while let Some(a) = iter.next() {
        match a.as_str() {
            "--mode=warn" => mode = Mode::Warn,
            "--mode=fail" => mode = Mode::Fail,
            "--mode" => match iter.next().as_deref() {
                Some("warn") => mode = Mode::Warn,
                Some("fail") => mode = Mode::Fail,
                other => bail!("--mode expects warn|fail, got {other:?}"),
            },
            other if other.starts_with("--budget=") => {
                budget = other
                    .strip_prefix("--budget=")
                    .unwrap()
                    .parse()
                    .context("--budget=N expects a positive integer")?;
            }
            "--budget" => {
                budget = iter
                    .next()
                    .context("--budget needs a value")?
                    .parse()
                    .context("--budget expects a positive integer")?;
            }
            "-h" | "--help" => {
                println!(
                    "usage: cargo run -p xtask -- check-file-budget [--mode=warn|fail] [--budget=N] [path...]"
                );
                std::process::exit(0);
            }
            other if other.starts_with("--") => bail!("unknown flag: {other}"),
            other => paths.push(PathBuf::from(other)),
        }
    }
    Ok((mode, budget, paths))
}

fn workspace_root() -> Result<PathBuf> {
    let manifest = std::env::var("CARGO_MANIFEST_DIR")
        .context("CARGO_MANIFEST_DIR not set; run via `cargo run -p xtask`")?;
    Ok(Path::new(&manifest)
        .parent()
        .ok_or_else(|| anyhow!("xtask manifest has no parent"))?
        .to_path_buf())
}

fn default_scan_roots(root: &Path) -> Vec<PathBuf> {
    vec![
        root.join("crates"),
        root.join("xtask").join("src"),
        root.join("apps").join("dashboard").join("src"),
    ]
}

const SKIP_DIRS: &[&str] = &[
    "target",
    "node_modules",
    "dist",
    "build",
    ".next",
    ".turbo",
    "out",
    "coverage",
];

fn walk_source_files(base: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    walk_inner(base, &mut out);
    out.sort();
    out
}

fn walk_inner(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if path.is_dir() {
            if SKIP_DIRS.contains(&name.as_ref()) || name.starts_with('.') {
                continue;
            }
            walk_inner(&path, out);
        } else if let Some(ext) = path.extension().and_then(|s| s.to_str()) {
            if matches!(ext, "rs" | "ts" | "tsx") {
                out.push(path);
            }
        }
    }
}

/// Decide whether a file represents prod content. Test/example/bench
/// directories and conventionally-named test files are excluded; the
/// remaining `.rs`/`.ts`/`.tsx` files are counted.
fn is_prod_source(rel: &Path) -> bool {
    let s = rel.to_string_lossy().replace('\\', "/");
    let name = rel.file_name().and_then(|s| s.to_str()).unwrap_or("");
    let ext = rel.extension().and_then(|s| s.to_str()).unwrap_or("");

    for component in s.split('/') {
        if matches!(component, "tests" | "examples" | "benches" | "__tests__") {
            return false;
        }
    }

    match ext {
        "rs" => !(name == "tests.rs" || name.ends_with("_tests.rs")),
        "ts" | "tsx" => {
            !(name.ends_with(&format!(".test.{ext}")) || name.ends_with(&format!(".spec.{ext}")))
        }
        _ => false,
    }
}

fn find_allow(src: &str) -> Option<String> {
    for line in src.lines() {
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix(ALLOW_PREFIX) {
            let reason = rest.trim();
            if !reason.is_empty() {
                return Some(reason.to_string());
            }
        }
    }
    None
}

fn build_bpe() -> Result<CoreBPE> {
    let mut encoder: FxHashMap<Vec<u8>, Rank> = FxHashMap::default();
    for (idx, line) in VENDORED_VOCAB.lines().enumerate() {
        let line_no = idx + 1;
        let mut parts = line.split(' ');
        let raw = parts
            .next()
            .with_context(|| format!("o200k_base.tiktoken line {line_no}: missing token"))?;
        let token = general_purpose::STANDARD
            .decode(raw)
            .with_context(|| format!("o200k_base.tiktoken line {line_no}: invalid base64"))?;
        let rank: Rank = parts
            .next()
            .with_context(|| format!("o200k_base.tiktoken line {line_no}: missing rank"))?
            .parse()
            .with_context(|| format!("o200k_base.tiktoken line {line_no}: bad rank"))?;
        encoder.insert(token, rank);
    }
    let mut special: FxHashMap<String, Rank> = FxHashMap::default();
    special.insert(ENDOFTEXT.to_string(), 199_999);
    special.insert(ENDOFPROMPT.to_string(), 200_018);
    CoreBPE::new(encoder, special, O200K_BASE_PAT_STR)
        .map_err(|e| anyhow!("build CoreBPE for o200k_base: {e}"))
}

/// Strip `#[cfg(test)]`-attributed top-level items (and `cfg(test)` mod
/// blocks nested one level deep). Falls back to the original source if
/// the file fails to parse — counting too much is preferable to silently
/// dropping the file.
fn strip_rust_cfg_test(src: &str) -> Result<String> {
    let file: SynFile = syn::parse_file(src)?;
    let mut excluded: Vec<std::ops::Range<usize>> = Vec::new();
    collect_cfg_test_ranges(&file.items, &mut excluded);
    if excluded.is_empty() {
        return Ok(src.to_string());
    }
    excluded.sort_by_key(|r| r.start);
    let mut merged: Vec<std::ops::Range<usize>> = Vec::with_capacity(excluded.len());
    for r in excluded {
        match merged.last_mut() {
            Some(last) if r.start <= last.end => last.end = last.end.max(r.end),
            _ => merged.push(r),
        }
    }
    let mut out = String::with_capacity(src.len());
    let mut cursor = 0usize;
    for r in &merged {
        if r.start > cursor && r.start <= src.len() {
            out.push_str(&src[cursor..r.start]);
        }
        cursor = cursor.max(r.end.min(src.len()));
    }
    if cursor < src.len() {
        out.push_str(&src[cursor..]);
    }
    Ok(out)
}

fn collect_cfg_test_ranges(items: &[Item], out: &mut Vec<std::ops::Range<usize>>) {
    for item in items {
        let attrs = item_attrs(item);
        if !attrs.is_empty() && attrs.iter().any(is_cfg_test_attr) {
            let attr_span = attrs[0].span().byte_range();
            let item_span = item.span().byte_range();
            let start = attr_span.start.min(item_span.start);
            let end = item_span.end.max(attr_span.end);
            if end > start {
                out.push(start..end);
            }
            continue;
        }
        if let Item::Mod(m) = item {
            if let Some((_, inner)) = &m.content {
                collect_cfg_test_ranges(inner, out);
            }
        }
    }
}

fn item_attrs(item: &Item) -> &[Attribute] {
    match item {
        Item::Const(i) => &i.attrs,
        Item::Enum(i) => &i.attrs,
        Item::ExternCrate(i) => &i.attrs,
        Item::Fn(i) => &i.attrs,
        Item::ForeignMod(i) => &i.attrs,
        Item::Impl(i) => &i.attrs,
        Item::Macro(i) => &i.attrs,
        Item::Mod(i) => &i.attrs,
        Item::Static(i) => &i.attrs,
        Item::Struct(i) => &i.attrs,
        Item::Trait(i) => &i.attrs,
        Item::TraitAlias(i) => &i.attrs,
        Item::Type(i) => &i.attrs,
        Item::Union(i) => &i.attrs,
        Item::Use(i) => &i.attrs,
        _ => &[],
    }
}

fn is_cfg_test_attr(attr: &Attribute) -> bool {
    if !attr.path().is_ident("cfg") {
        return false;
    }
    match &attr.meta {
        Meta::List(list) => {
            list.tokens
                .to_string()
                .split_whitespace()
                .collect::<String>()
                == "test"
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allow_marker_is_parsed() {
        let src = "// budget-allow: imported third-party fixture\nfn x() {}\n";
        assert_eq!(
            find_allow(src).as_deref(),
            Some("imported third-party fixture")
        );
    }

    #[test]
    fn allow_marker_requires_reason() {
        assert_eq!(find_allow("// budget-allow:\n").as_deref(), None);
        assert_eq!(find_allow("// budget-allow:   \n").as_deref(), None);
    }

    #[test]
    fn allow_marker_can_be_indented_anywhere() {
        let src = "fn a() {}\n\n   // budget-allow: see #999 follow-up\n";
        assert_eq!(find_allow(src).as_deref(), Some("see #999 follow-up"));
    }

    #[test]
    fn strip_removes_cfg_test_block_at_eof() {
        let src = r#"
fn prod() {}

#[cfg(test)]
mod tests {
    #[test]
    fn t() { let _x = 1; }
}
"#;
        let stripped = strip_rust_cfg_test(src).expect("parse");
        assert!(stripped.contains("fn prod"));
        assert!(!stripped.contains("#[cfg(test)]"));
        assert!(!stripped.contains("fn t"));
    }

    #[test]
    fn strip_keeps_prod_when_no_cfg_test() {
        let src = "fn a() {}\nfn b() {}\n";
        let stripped = strip_rust_cfg_test(src).expect("parse");
        assert_eq!(stripped, src);
    }

    #[test]
    fn cfg_test_attr_match_is_strict() {
        // Synthesized attrs to exercise the matcher.
        let cfg_test: Attribute = syn::parse_quote!(#[cfg(test)]);
        let cfg_unix: Attribute = syn::parse_quote!(#[cfg(unix)]);
        let cfg_test_ws: Attribute = syn::parse_quote!(#[cfg( test )]);
        let allow: Attribute = syn::parse_quote!(#[allow(dead_code)]);

        assert!(is_cfg_test_attr(&cfg_test));
        assert!(is_cfg_test_attr(&cfg_test_ws));
        assert!(!is_cfg_test_attr(&cfg_unix));
        assert!(!is_cfg_test_attr(&allow));
    }

    #[test]
    fn prod_classification() {
        assert!(is_prod_source(Path::new(
            "crates/forge/src/core/pipeline.rs"
        )));
        assert!(!is_prod_source(Path::new(
            "crates/foo/tests/integration.rs"
        )));
        assert!(!is_prod_source(Path::new(
            "crates/synodic/src/core/storage/tests.rs"
        )));
        assert!(!is_prod_source(Path::new("crates/foo/src/x_tests.rs")));
        assert!(is_prod_source(Path::new("apps/dashboard/src/lib/api.ts")));
        assert!(!is_prod_source(Path::new(
            "apps/dashboard/tests/smoke/x.test.ts"
        )));
        assert!(!is_prod_source(Path::new(
            "apps/dashboard/src/__tests__/x.tsx"
        )));
    }

    #[test]
    fn token_count_is_stable_for_known_input() {
        let bpe = build_bpe().expect("vocab");
        // o200k_base produces stable counts; "hello world" is a tiny canary.
        let n = bpe.encode_ordinary("hello world").len();
        assert!((1..=10).contains(&n), "unexpected token count: {n}");
    }
}
