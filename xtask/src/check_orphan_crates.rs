//! Orphan-crate lint (spec #275) — Occam's-razor projection of "no
//! dangling wires" onto the crate graph.
//!
//! For every workspace library crate (one that exposes `src/lib.rs` and
//! is **not** a `[[bin]]`-shipping subsystem), count how many other
//! workspace crates declare it as a path dep. Zero in-tree reverse deps
//! → violation.
//!
//! Bin-shipping crates (`onsager`, `stiglab`, `synodic`, `ising`,
//! `onsager-portal`, `onsager-trigger`, `onsager-scheduler`) are
//! excluded — they're top-level apps; reverse-dep count doesn't apply.
//!
//! ## Escape hatch
//!
//! A `// occam-allow: <non-empty reason>` line anywhere in `src/lib.rs`
//! exempts the crate. Mirrors `seam-allow` / `budget-allow`. Use
//! sparingly — every allow is a wire connected at one end.
//!
//! ## Modes
//!
//! - `--mode=warn` (default): print violations, exit 0.
//! - `--mode=fail`: exit non-zero on any unallowed violation.
//!
//! Landing in warn mode per spec #275; ratchet to fail in a follow-up.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};

const ALLOW_PREFIX: &str = "// occam-allow:";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Warn,
    Fail,
}

pub fn run(args: Vec<String>) -> Result<()> {
    let mode = parse_mode(&args)?;
    let root = workspace_root()?;

    let crates = discover_crates(&root)?;
    let reverse_deps = build_reverse_dep_map(&crates);

    let mut violations: Vec<(String, PathBuf)> = Vec::new();
    let mut allowed: Vec<(String, String)> = Vec::new();

    for c in &crates {
        if !c.is_library() {
            continue;
        }
        let count = reverse_deps.get(c.name.as_str()).copied().unwrap_or(0);
        if count > 0 {
            continue;
        }
        let lib_rs = c.dir.join("src").join("lib.rs");
        if let Some(reason) = read_occam_allow(&lib_rs)? {
            allowed.push((c.name.clone(), reason));
            continue;
        }
        violations.push((c.name.clone(), c.dir.clone()));
    }

    if !allowed.is_empty() {
        eprintln!("orphan-crate occam-allow exemptions:");
        for (name, reason) in &allowed {
            eprintln!("  {name} — allowed: {reason}");
        }
        eprintln!();
    }

    if violations.is_empty() {
        println!(
            "check-orphan-crates: clean ({} library crate(s) scanned, {} allowed exemption(s))",
            crates.iter().filter(|c| c.is_library()).count(),
            allowed.len()
        );
        return Ok(());
    }

    eprintln!(
        "check-orphan-crates: {} violation(s) — library crates with zero in-tree reverse deps:",
        violations.len()
    );
    for (name, dir) in &violations {
        eprintln!("  {name} ({})", dir.display());
    }
    eprintln!();
    eprintln!("See spec #275 (Occam's-razor lints). To exempt a crate, add");
    eprintln!("`// occam-allow: <reason>` to its `src/lib.rs`.");

    match mode {
        Mode::Warn => {
            eprintln!("(warn mode — not failing)");
            Ok(())
        }
        Mode::Fail => bail!("orphan-crate lint failed"),
    }
}

fn parse_mode(args: &[String]) -> Result<Mode> {
    let mut mode = Mode::Warn;
    for arg in args {
        match arg.as_str() {
            "--mode=warn" => mode = Mode::Warn,
            "--mode=fail" => mode = Mode::Fail,
            other => bail!("unknown flag for check-orphan-crates: {other}"),
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

#[derive(Debug)]
struct CrateInfo {
    name: String,
    dir: PathBuf,
    has_lib: bool,
    has_bin: bool,
    path_deps: Vec<String>,
}

impl CrateInfo {
    /// "Pure library" means it ships no binary. Bin-shipping subsystems
    /// (`stiglab`, `synodic`, `ising`, `onsager-portal`, `onsager`,
    /// `onsager-trigger`, `onsager-scheduler`) are top-level apps and
    /// don't need a reverse-dep count; their `src/lib.rs` (when present)
    /// is the subsystem's test surface.
    fn is_library(&self) -> bool {
        self.has_lib && !self.has_bin
    }
}

fn discover_crates(root: &Path) -> Result<Vec<CrateInfo>> {
    let crates_dir = root.join("crates");
    let mut out = Vec::new();
    for entry in
        std::fs::read_dir(&crates_dir).with_context(|| format!("read {}", crates_dir.display()))?
    {
        let entry = entry?;
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }
        let manifest = dir.join("Cargo.toml");
        if !manifest.is_file() {
            continue;
        }
        let info = parse_manifest(&dir, &manifest)?;
        out.push(info);
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

fn parse_manifest(dir: &Path, manifest: &Path) -> Result<CrateInfo> {
    let text = std::fs::read_to_string(manifest)
        .with_context(|| format!("read {}", manifest.display()))?;
    let parsed: toml::Value =
        toml::from_str(&text).with_context(|| format!("parse {}", manifest.display()))?;

    let name = parsed
        .get("package")
        .and_then(|p| p.get("name"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("{} has no [package].name", manifest.display()))?
        .to_string();

    let has_lib =
        dir.join("src").join("lib.rs").is_file() || parsed.get("lib").is_some_and(|t| t.is_table());
    let has_bin = dir.join("src").join("main.rs").is_file()
        || parsed
            .get("bin")
            .and_then(|t| t.as_array())
            .is_some_and(|a| !a.is_empty());

    let mut path_deps = Vec::new();
    if let Some(deps) = parsed.get("dependencies").and_then(|d| d.as_table()) {
        for (dep_name, val) in deps {
            if let Some(t) = val.as_table()
                && t.get("path").is_some()
            {
                path_deps.push(dep_name.clone());
            }
        }
    }

    Ok(CrateInfo {
        name,
        dir: dir.to_path_buf(),
        has_lib,
        has_bin,
        path_deps,
    })
}

fn build_reverse_dep_map(crates: &[CrateInfo]) -> BTreeMap<&str, usize> {
    let names: BTreeSet<&str> = crates.iter().map(|c| c.name.as_str()).collect();
    let mut counts: BTreeMap<&str, usize> = names.iter().map(|n| (*n, 0)).collect();
    for c in crates {
        for dep in &c.path_deps {
            if names.contains(dep.as_str())
                && let Some(slot) = counts.get_mut(dep.as_str())
            {
                *slot += 1;
            }
        }
    }
    counts
}

fn read_occam_allow(path: &Path) -> Result<Option<String>> {
    if !path.is_file() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    Ok(parse_occam_allow(&text))
}

/// Extract a non-empty `// occam-allow: <reason>` body from `s`. The
/// reason text must be non-empty after trimming — empty reasons are
/// rejected at lint time, matching `seam-allow` / `budget-allow`.
pub(crate) fn parse_occam_allow(s: &str) -> Option<String> {
    for line in s.lines() {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn occam_allow_parses_non_empty_reason() {
        assert_eq!(
            parse_occam_allow("// occam-allow: waiting on #361"),
            Some("waiting on #361".to_string())
        );
        assert_eq!(
            parse_occam_allow("    // occam-allow:  trailing-spaces  "),
            Some("trailing-spaces".to_string())
        );
    }

    #[test]
    fn occam_allow_rejects_empty_reason() {
        assert_eq!(parse_occam_allow("// occam-allow:"), None);
        assert_eq!(parse_occam_allow("// occam-allow:   "), None);
    }

    #[test]
    fn occam_allow_returns_none_when_absent() {
        assert_eq!(parse_occam_allow("//! crate doc\nfn x() {}"), None);
    }

    #[test]
    fn library_classification_excludes_bin_crates() {
        let info = CrateInfo {
            name: "stiglab".into(),
            dir: PathBuf::from("/tmp/stiglab"),
            has_lib: true,
            has_bin: true,
            path_deps: vec![],
        };
        assert!(!info.is_library(), "bin-shipping crate is not a library");
        let info = CrateInfo {
            name: "onsager-spine".into(),
            dir: PathBuf::from("/tmp/spine"),
            has_lib: true,
            has_bin: false,
            path_deps: vec![],
        };
        assert!(info.is_library(), "pure lib crate is a library");
    }

    #[test]
    fn reverse_dep_count_aggregates_across_workspace() {
        let crates = vec![
            CrateInfo {
                name: "a".into(),
                dir: PathBuf::from("/a"),
                has_lib: true,
                has_bin: false,
                path_deps: vec!["b".into(), "c".into()],
            },
            CrateInfo {
                name: "b".into(),
                dir: PathBuf::from("/b"),
                has_lib: true,
                has_bin: false,
                path_deps: vec!["c".into()],
            },
            CrateInfo {
                name: "c".into(),
                dir: PathBuf::from("/c"),
                has_lib: true,
                has_bin: false,
                path_deps: vec![],
            },
            CrateInfo {
                name: "orphan".into(),
                dir: PathBuf::from("/orphan"),
                has_lib: true,
                has_bin: false,
                path_deps: vec![],
            },
        ];
        let counts = build_reverse_dep_map(&crates);
        assert_eq!(counts["a"], 0);
        assert_eq!(counts["b"], 1);
        assert_eq!(counts["c"], 2);
        assert_eq!(counts["orphan"], 0);
    }
}
