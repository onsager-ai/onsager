//! `cargo run -p xtask -- check-detail-page-tabs`
//!
//! ADR 0021 invariant 3 — `<Tabs>` is not primary IA on detail pages.
//! Tabs smuggle the v0.1 "node attribute = top-level surface" mental
//! model; a detail page is one document of labelled sections, and
//! orthogonal *views of the same data* (e.g. a Provenance axis switch)
//! use a segmented control, not the Tabs primitive.
//!
//! This lint scans `apps/dashboard/src/pages/*DetailPage.tsx` and fails
//! when one imports the Tabs primitive or renders `<Tabs …>` JSX.
//!
//! Escape hatch — inline `// tabs-allow: <reason>` on the offending
//! line. Reason text is mandatory and grep-able, mirroring the
//! `// seam-allow:` / `// metaphor-allow:` shape. Use it for a genuine
//! orthogonal-view tab *inside* a section (the ADR permits that), never
//! to restore tabs as the page's primary navigation.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};

const SCAN_DIR: &str = "apps/dashboard/src/pages";

/// Signals that a detail page is leaning on the Tabs primitive. The
/// import is the strongest signal ("may not import `<Tabs>`"); the JSX
/// tags catch a re-export or aliased import.
const NEEDLES: &[&str] = &[
    "@/components/ui/tabs",
    "<Tabs",
    "<TabsList",
    "<TabsTrigger",
    "<TabsContent",
];

pub fn run() -> Result<()> {
    let root = workspace_root()?;
    let scan_dir = root.join(SCAN_DIR);
    let files = collect_detail_pages(&scan_dir)?;

    let mut violations: Vec<Violation> = Vec::new();
    let mut inline_allowed: Vec<Violation> = Vec::new();

    for file in &files {
        let rel = file.strip_prefix(&root).unwrap_or(file);
        let text =
            std::fs::read_to_string(file).with_context(|| format!("read {}", file.display()))?;
        for hit in scan_text(&text) {
            let v = Violation {
                path: rel.to_path_buf(),
                line_no: hit.line_no,
                needle: hit.needle,
                snippet: hit.snippet,
            };
            if has_inline_allow(&v.snippet) {
                inline_allowed.push(v);
            } else {
                violations.push(v);
            }
        }
    }

    if !inline_allowed.is_empty() {
        eprintln!("check-detail-page-tabs: inline tabs-allow exemptions:");
        for v in &inline_allowed {
            eprintln!(
                "  {}:{} [{}] {}",
                v.path.display(),
                v.line_no,
                v.needle,
                v.snippet.trim()
            );
        }
        eprintln!();
    }

    if violations.is_empty() {
        println!(
            "check-detail-page-tabs: clean ({} detail page(s) scanned, {} inline-allowed)",
            files.len(),
            inline_allowed.len()
        );
        return Ok(());
    }

    eprintln!(
        "check-detail-page-tabs: {} tab-as-primary-IA violation(s):",
        violations.len()
    );
    for v in &violations {
        eprintln!(
            "  {}:{} [{}] {}",
            v.path.display(),
            v.line_no,
            v.needle,
            v.snippet.trim()
        );
    }
    eprintln!();
    eprintln!("ADR 0021 inv. 3: `<Tabs>` is not primary IA on detail pages. Organize the page");
    eprintln!("as labelled sections; for an orthogonal view of the *same* data (e.g. a");
    eprintln!("Provenance axis switch) use a segmented control, or add an inline");
    eprintln!("`// tabs-allow: <reason>` on the line for a genuine in-section tab.");
    bail!("detail-page-tabs lint failed");
}

#[derive(Debug)]
struct Violation {
    path: PathBuf,
    line_no: usize,
    needle: &'static str,
    snippet: String,
}

#[derive(Debug, PartialEq, Eq)]
struct Hit {
    needle: &'static str,
    line_no: usize,
    snippet: String,
}

fn scan_text(text: &str) -> Vec<Hit> {
    let mut out: Vec<Hit> = Vec::new();
    for (idx, line) in text.lines().enumerate() {
        for needle in NEEDLES {
            if line.contains(needle) {
                out.push(Hit {
                    needle,
                    line_no: idx + 1,
                    snippet: line.to_string(),
                });
            }
        }
    }
    out
}

fn has_inline_allow(snippet: &str) -> bool {
    let idx = match snippet.find("tabs-allow:") {
        Some(i) => i,
        None => return false,
    };
    let reason = snippet[idx + "tabs-allow:".len()..].trim();
    let trimmed = reason.trim_end_matches("*/").trim();
    !trimmed.is_empty()
}

fn workspace_root() -> Result<PathBuf> {
    let manifest = std::env::var("CARGO_MANIFEST_DIR")
        .context("CARGO_MANIFEST_DIR not set; run via `cargo run -p xtask`")?;
    Ok(Path::new(&manifest)
        .parent()
        .ok_or_else(|| anyhow!("xtask manifest has no parent"))?
        .to_path_buf())
}

/// `pages/*DetailPage.tsx` files only. The IA rule is about *detail*
/// pages; list/index pages legitimately use tabs (e.g. status chips are
/// a different primitive, but a future tabbed list view is out of scope
/// for this rule).
fn collect_detail_pages(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    if dir.is_dir() {
        for entry in
            std::fs::read_dir(dir).with_context(|| format!("read_dir {}", dir.display()))?
        {
            let path = entry?.path();
            if !path.is_file() {
                continue;
            }
            let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
            if name.ends_with("DetailPage.tsx") {
                out.push(path);
            }
        }
    }
    out.sort();
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tabs_import_is_a_hit() {
        let hits = scan_text("import { Tabs } from \"@/components/ui/tabs\"");
        assert!(hits.iter().any(|h| h.needle == "@/components/ui/tabs"));
    }

    #[test]
    fn tabs_jsx_is_a_hit() {
        let hits = scan_text("      <TabsList variant=\"line\">");
        assert!(hits.iter().any(|h| h.needle == "<TabsList"));
    }

    #[test]
    fn segmented_control_buttons_are_clean() {
        let hits = scan_text("        <Button variant={axis === a.key ? \"default\" : \"ghost\"}>");
        assert!(hits.is_empty());
    }

    #[test]
    fn inline_allow_exempts_a_line() {
        let snippet = r#"  <Tabs> // tabs-allow: provenance axis is an orthogonal view"#;
        assert!(has_inline_allow(snippet));
    }

    #[test]
    fn inline_allow_requires_a_reason() {
        let snippet = r#"  <Tabs> // tabs-allow: "#;
        assert!(!has_inline_allow(snippet));
    }

    #[test]
    fn live_dashboard_passes_lint() {
        // End-to-end: run the lint against the in-tree detail pages. Its
        // purpose is to keep this check green; a failure means a detail
        // page reintroduced tabs as primary IA (ADR 0021 inv. 3).
        super::run().expect("detail-page-tabs lint must pass on main");
    }
}
