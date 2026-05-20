//! Spec #298 — drift check for `apps/dashboard/src/lib/api/generated/`.
//!
//! The dashboard's API types are generated from `onsager-portal` and
//! `onsager-spine` serde structs via `ts-rs` at `cargo test` time
//! (spine joined the cascade with spec #434 — `TriggerKind` and its
//! variant tree). The generated files are committed so the
//! dashboard's `pnpm tsc --noEmit` can resolve them without a prior
//! Rust build. This check enforces "what's committed == what the
//! current Rust would emit": snapshot the dir, regenerate via
//! `cargo test -p onsager-portal -p onsager-spine --lib
//! export_bindings`, and bail on any add / remove / content change.
//!
//! Same shape as `gen-event-docs --check`: hard-fail with a
//! reproducible command in the error message.

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

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
        "{} is out of date.\n\nRegenerate with:\n    cargo test -p onsager-portal -p onsager-spine -p synodic --lib export_bindings\n\nThen commit the result. Differences:\n",
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
    // Drop `--quiet` so the harness summary ("test result: ok. N passed; …")
    // is in stdout — we parse N below to defend against the filter matching
    // zero tests (e.g. ts-rs renames its generated test prefix, or someone
    // drops `#[ts(export)]` from every type). Without that guard the drift
    // check would silently pass while doing no actual regeneration.
    // Portal, spine, and synodic all host `#[ts(export)]` types whose
    // generated bindings land in `GENERATED_DIR` (spine joined the
    // cascade with spec #434 — `TriggerKind` and its variant tree;
    // synodic joined with spec #441 — `GovernanceEvent`). Each `-p`
    // runs its own libtest harness invocation, so we drive them in
    // sequence and require at least one matched test per crate.
    for pkg in ["onsager-portal", "onsager-spine", "synodic"] {
        let output = Command::new("cargo")
            .args(["test", "-p", pkg, "--lib", "export_bindings"])
            .current_dir(root)
            .stdin(Stdio::null())
            .output()
            .with_context(|| format!("spawn `cargo test -p {pkg} --lib export_bindings`"))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stdout = String::from_utf8_lossy(&output.stdout);
            bail!(
                "`cargo test -p {pkg} export_bindings` failed (exit {}):\n--- stdout ---\n{}\n--- stderr ---\n{}",
                output.status,
                stdout.trim(),
                stderr.trim()
            );
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        let passed = parse_test_passed(&stdout).with_context(|| {
            format!(
                "could not find `test result: ok. N passed` summary in cargo test output for {pkg} — \
                 the harness format may have changed"
            )
        })?;
        if passed == 0 {
            bail!(
                "`cargo test -p {pkg} ... export_bindings` matched 0 tests — the ts-rs auto-generated \
                 `export_bindings_*` tests didn't run, so no regeneration happened for {pkg}.\n\
                 Check that `#[ts(export)]` is still present on its serde structs, and \
                 that ts-rs hasn't renamed its generated test prefix.\n\n\
                 cargo test output:\n{stdout}"
            );
        }
    }
    Ok(())
}

/// Parse the cargo test harness summary line for the passed count.
/// Format (libtest stable): `test result: ok. N passed; M failed; …`.
fn parse_test_passed(stdout: &str) -> Option<usize> {
    stdout.lines().find_map(|line| {
        let rest = line.trim().strip_prefix("test result: ok. ")?;
        let n_str = rest.split(" passed").next()?;
        n_str.trim().parse::<usize>().ok()
    })
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

#[cfg(test)]
mod tests {
    use super::parse_test_passed;

    #[test]
    fn parses_libtest_summary() {
        let stdout = "\
running 4 tests
test workflow::export_bindings_gatekind ... ok

test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 206 filtered out; finished in 0.17s
";
        assert_eq!(parse_test_passed(stdout), Some(4));
    }

    #[test]
    fn parses_zero_passed() {
        let stdout = "\
running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 210 filtered out; finished in 0.00s
";
        assert_eq!(parse_test_passed(stdout), Some(0));
    }

    #[test]
    fn returns_none_when_no_summary_line() {
        assert_eq!(
            parse_test_passed("compiling onsager-portal\nerror: ..."),
            None
        );
    }
}
