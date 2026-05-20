//! Spec #298 — drift check for `apps/dashboard/src/lib/api/generated/`.
//!
//! The dashboard's API types are generated from portal's serde structs
//! via `ts-rs` at `cargo test` time. The generated files are committed
//! so the dashboard's `pnpm tsc --noEmit` can resolve them without a
//! prior Rust build. This check enforces "what's committed == what the
//! current Rust would emit": snapshot the dir, regenerate via
//! `cargo test -p onsager-portal --lib export_bindings`, and bail on
//! any add / remove / content change.
//!
//! Same shape as `gen-event-docs --check`: hard-fail with a
//! reproducible command in the error message.

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

const GENERATED_DIR: &str = "apps/dashboard/src/lib/api/generated";

pub fn run() -> Result<()> {
    let root = workspace_root()?;
    let gen_dir = root.join(GENERATED_DIR);

    let before = snapshot(&gen_dir)
        .with_context(|| format!("snapshot {} before regenerate", gen_dir.display()))?;

    regenerate(&root)?;

    let after = snapshot(&gen_dir)
        .with_context(|| format!("snapshot {} after regenerate", gen_dir.display()))?;

    let drift = diff(&before, &after);
    if drift.is_empty() {
        println!(
            "check-generated-types: {} file(s) up to date in {}",
            after.len(),
            GENERATED_DIR
        );
        return Ok(());
    }

    let mut msg = format!(
        "{} is out of date.\n\nRegenerate with:\n    cargo test -p onsager-portal --lib export_bindings\n\nThen commit the result. Differences:\n",
        GENERATED_DIR
    );
    for line in &drift {
        msg.push_str("  ");
        msg.push_str(line);
        msg.push('\n');
    }
    bail!("{msg}");
}

fn workspace_root() -> Result<PathBuf> {
    let manifest = std::env::var("CARGO_MANIFEST_DIR")
        .context("CARGO_MANIFEST_DIR not set; run via `cargo run -p xtask`")?;
    Ok(Path::new(&manifest)
        .parent()
        .context("xtask manifest has no parent")?
        .to_path_buf())
}

fn snapshot(dir: &Path) -> Result<BTreeMap<PathBuf, String>> {
    let mut out: BTreeMap<PathBuf, String> = BTreeMap::new();
    if !dir.exists() {
        return Ok(out);
    }
    walk(dir, dir, &mut out)?;
    Ok(out)
}

fn walk(root: &Path, here: &Path, out: &mut BTreeMap<PathBuf, String>) -> Result<()> {
    for entry in std::fs::read_dir(here).with_context(|| format!("read_dir {}", here.display()))? {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            walk(root, &path, out)?;
            continue;
        }
        if !file_type.is_file() {
            continue;
        }
        // Only track .ts files — anything else under the generated dir is
        // outside the SSOT contract and shouldn't gate the check.
        if path.extension().and_then(OsStr::to_str) != Some("ts") {
            continue;
        }
        let rel = path
            .strip_prefix(root)
            .with_context(|| format!("strip_prefix {} from {}", root.display(), path.display()))?
            .to_path_buf();
        let body =
            std::fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
        out.insert(rel, body);
    }
    Ok(())
}

fn regenerate(root: &Path) -> Result<()> {
    let status = Command::new("cargo")
        .args([
            "test",
            "-p",
            "onsager-portal",
            "--lib",
            "--quiet",
            "export_bindings",
        ])
        .current_dir(root)
        .status()
        .context("spawn `cargo test -p onsager-portal --lib export_bindings`")?;
    if !status.success() {
        bail!("`cargo test export_bindings` failed (exit {status})");
    }
    Ok(())
}

fn diff(before: &BTreeMap<PathBuf, String>, after: &BTreeMap<PathBuf, String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for (path, body) in after {
        match before.get(path) {
            None => out.push(format!("added: {}", path.display())),
            Some(prev) if prev != body => out.push(format!("changed: {}", path.display())),
            _ => {}
        }
    }
    for path in before.keys() {
        if !after.contains_key(path) {
            out.push(format!("removed: {}", path.display()));
        }
    }
    out.sort();
    out
}
