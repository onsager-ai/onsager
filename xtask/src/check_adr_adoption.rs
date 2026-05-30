//! ADR-adoption lint (ADR 0024 / 0025, spec #506 Phase 4) — the "no
//! dangling wires" value applied to the ADR corpus.
//!
//! An `Accepted` ADR is a decision the codebase claims to have adopted.
//! Its **Adoption checklist** is the contract: unchecked `- [ ]` items
//! are work the ADR says is still owed. An Accepted ADR carrying
//! unchecked items is either (a) genuinely in flight — which the ADR
//! should declare — or (b) drift: a decision marked landed whose
//! follow-through silently stalled. This lint surfaces (b).
//!
//! For every `docs/adr/NNNN-*.md` whose metadata block carries
//! `Status: Accepted`, count the unchecked items in its `## Adoption
//! checklist` section. Any unchecked item is a finding — unless the ADR
//! opts out via `Adoption: ongoing` (see escape hatch).
//!
//! ## Escape hatch
//!
//! An ADR whose metadata block opts out via an `Adoption: ongoing`
//! marker is exempt — its implementation is acknowledged as still in
//! flight. Two forms are recognized, both case-insensitively: the
//! rendered metadata line `- **Adoption**: ongoing` (the normal shape)
//! and a bare `Adoption: ongoing`. Flip it to `Adoption: enforced` (or
//! drop the line) and tick the boxes as the work lands.
//!
//! ## Modes
//!
//! - `--mode=warn` (default): print findings, exit 0.
//! - `--mode=fail`: exit non-zero on any finding.
//!
//! Lands in warn mode (warn-then-ratchet, like the Occam's-razor lints
//! of spec #275); a follow-up ratchets to fail once the ADR corpus is
//! clean.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};

const ADR_DIR: &str = "docs/adr";
const ADOPTION_HEADING: &str = "## Adoption checklist";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Warn,
    Fail,
}

#[derive(Debug)]
struct Finding {
    adr: String,
    unchecked: usize,
    /// The first unchecked item's text, for a human-legible pointer.
    first_unchecked: String,
}

pub fn run(args: Vec<String>) -> Result<()> {
    let mode = parse_mode(&args)?;
    let root = workspace_root()?;
    let dir = root.join(ADR_DIR);

    let files = collect_adr_files(&dir)?;

    let mut findings: Vec<Finding> = Vec::new();
    let mut accepted = 0usize;
    let mut exempt: Vec<String> = Vec::new();

    for file in &files {
        let rel = file
            .strip_prefix(&root)
            .unwrap_or(file)
            .to_string_lossy()
            .to_string();
        let text = std::fs::read_to_string(file).with_context(|| format!("read {rel}"))?;

        if !is_accepted(&text) {
            continue;
        }
        accepted += 1;

        if is_adoption_ongoing(&text) {
            exempt.push(rel);
            continue;
        }

        let unchecked = unchecked_adoption_items(&text);
        if !unchecked.is_empty() {
            findings.push(Finding {
                adr: rel,
                unchecked: unchecked.len(),
                first_unchecked: unchecked[0].clone(),
            });
        }
    }

    if !exempt.is_empty() {
        eprintln!("check-adr-adoption: Adoption: ongoing exemptions:");
        for adr in &exempt {
            eprintln!("  {adr}");
        }
        eprintln!();
    }

    if findings.is_empty() {
        println!(
            "check-adr-adoption: clean ({accepted} Accepted ADR(s) scanned, {} ongoing-exempt)",
            exempt.len()
        );
        return Ok(());
    }

    eprintln!(
        "check-adr-adoption: {} Accepted ADR(s) with unchecked adoption items:",
        findings.len()
    );
    for f in &findings {
        eprintln!(
            "  {} — {} unchecked (e.g. \"{}\")",
            f.adr,
            f.unchecked,
            f.first_unchecked.trim()
        );
    }
    eprintln!();
    eprintln!("An Accepted ADR's adoption checklist is a contract: tick the items as");
    eprintln!("the work lands, or mark the ADR `Adoption: ongoing` in its metadata block");
    eprintln!("while implementation is genuinely in flight.");

    match mode {
        Mode::Warn => {
            eprintln!("(warn mode — not failing)");
            Ok(())
        }
        Mode::Fail => bail!("adr-adoption lint failed"),
    }
}

/// True when the metadata block declares `Status: ... Accepted ...`.
/// Tolerates the `- **Status**: Accepted (2026-05-22) — note` shape the
/// ADRs use; matches on the `Status` line specifically so the word
/// "Accepted" elsewhere in prose doesn't count.
fn is_accepted(text: &str) -> bool {
    text.lines()
        .filter_map(status_value)
        .any(|v| v.to_lowercase().contains("accepted"))
}

/// Return the value of a `- **Status**: <value>` metadata line, if this
/// line is one.
fn status_value(line: &str) -> Option<&str> {
    let l = line.trim_start();
    let rest = l.strip_prefix("- **Status**:")?;
    Some(rest.trim())
}

/// True when the metadata block opts out via `Adoption: ongoing`
/// (case-insensitive). Matches the rendered `- **Adoption**: ongoing`
/// line as well as a bare `Adoption: ongoing`.
fn is_adoption_ongoing(text: &str) -> bool {
    let lower = text.to_lowercase();
    lower.contains("adoption: ongoing") || lower.contains("adoption**: ongoing")
}

/// Collect the unchecked `- [ ]` items inside the `## Adoption checklist`
/// section (up to the next `## ` heading or EOF). Checked items (`- [x]`)
/// and nested continuation lines are ignored.
fn unchecked_adoption_items(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut in_section = false;
    for line in text.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("## ") {
            // Entering or leaving a section. The adoption section ends at
            // the next top-level heading.
            in_section = trimmed == ADOPTION_HEADING;
            continue;
        }
        if !in_section {
            continue;
        }
        if let Some(item) = trimmed.strip_prefix("- [ ]") {
            out.push(item.trim().to_string());
        }
    }
    out
}

fn parse_mode(args: &[String]) -> Result<Mode> {
    let mut mode = Mode::Warn;
    for arg in args {
        match arg.as_str() {
            "--mode=warn" => mode = Mode::Warn,
            "--mode=fail" => mode = Mode::Fail,
            other => bail!("unknown flag for check-adr-adoption: {other}"),
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

/// All `NNNN-*.md` ADR files (numbered), sorted. The `README.md` index
/// is skipped — it carries no `Status` line of its own.
fn collect_adr_files(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    if !dir.is_dir() {
        bail!("ADR dir not found: {}", dir.display());
    }
    for entry in std::fs::read_dir(dir).with_context(|| format!("read_dir {}", dir.display()))? {
        let entry = entry?;
        let path = entry.path();
        let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
        if path.extension().and_then(|s| s.to_str()) != Some("md") {
            continue;
        }
        // ADR files are `NNNN-slug.md` — exactly four leading digits
        // then a hyphen. The `README.md` index and any other stray
        // markdown are skipped.
        if is_adr_filename(name) {
            out.push(path);
        }
    }
    out.sort();
    Ok(out)
}

/// True for the ADR filename shape `NNNN-slug.md` — exactly four leading
/// ASCII digits followed by a hyphen. The extension is checked by the
/// caller; this only vets the numeric prefix so `README.md`, `1.md`, or
/// a hypothetical `notes.md` are rejected.
fn is_adr_filename(name: &str) -> bool {
    let bytes = name.as_bytes();
    bytes.len() > 4 && bytes[..4].iter().all(u8::is_ascii_digit) && bytes[4] == b'-'
}

#[cfg(test)]
mod tests {
    use super::*;

    const ACCEPTED_WITH_UNCHECKED: &str = "\
# ADR 9001 — Test

- **Status**: Accepted
- **Identity impact**: no

## Adoption checklist

- [x] Decision recorded.
- [ ] Land the thing.
- [ ] Land the other thing.

## Out of scope
- [ ] not counted (different section)
";

    const ONGOING: &str = "\
# ADR 9002 — Test

- **Status**: Accepted
- **Adoption**: ongoing

## Adoption checklist

- [ ] Still in flight.
";

    const PROPOSED: &str = "\
# ADR 9003 — Test

- **Status**: Proposed

## Adoption checklist

- [ ] Not adopted yet, but not Accepted either.
";

    const CLEAN: &str = "\
# ADR 9004 — Test

- **Status**: Accepted (2026-05-22) — note

## Adoption checklist

- [x] Decision recorded.
- [x] Landed.
";

    #[test]
    fn accepted_status_detected_with_parenthetical() {
        assert!(is_accepted(CLEAN));
        assert!(is_accepted(ACCEPTED_WITH_UNCHECKED));
    }

    #[test]
    fn proposed_is_not_accepted() {
        assert!(!is_accepted(PROPOSED));
    }

    #[test]
    fn accepted_word_in_prose_does_not_count() {
        let body = "- **Status**: Proposed\n\nWe Accepted nothing in the body.";
        assert!(!is_accepted(body));
    }

    #[test]
    fn unchecked_items_only_from_adoption_section() {
        let items = unchecked_adoption_items(ACCEPTED_WITH_UNCHECKED);
        // The two in-section unchecked items, not the Out-of-scope one.
        assert_eq!(items.len(), 2);
        assert_eq!(items[0], "Land the thing.");
    }

    #[test]
    fn checked_items_are_not_counted() {
        assert!(unchecked_adoption_items(CLEAN).is_empty());
    }

    #[test]
    fn ongoing_exemption_matches_rendered_metadata() {
        assert!(is_adoption_ongoing(ONGOING));
    }

    #[test]
    fn ongoing_exemption_absent_by_default() {
        assert!(!is_adoption_ongoing(ACCEPTED_WITH_UNCHECKED));
    }

    #[test]
    fn adr_filename_predicate() {
        assert!(is_adr_filename("0024-workflow-lifecycle.md"));
        assert!(is_adr_filename("0001-event-bus.md"));
        // Rejected: the index, short/garbled prefixes, no hyphen.
        assert!(!is_adr_filename("README.md"));
        assert!(!is_adr_filename("1.md"));
        assert!(!is_adr_filename("12-x.md"));
        assert!(!is_adr_filename("9abc-x.md"));
        assert!(!is_adr_filename("0024_underscore.md"));
    }

    #[test]
    fn live_adr_corpus_is_scannable() {
        // The lint must at least parse every in-tree ADR without error.
        // It runs in warn mode here so unchecked items on existing
        // Accepted ADRs don't fail the unit test; the CI invocation owns
        // the warn/fail decision.
        super::run(vec!["--mode=warn".to_string()]).expect("lint must run over the ADR corpus");
    }
}
