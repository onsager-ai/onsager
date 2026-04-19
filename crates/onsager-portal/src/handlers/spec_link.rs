//! Spec-link parsing — pull `Closes #N`, `Part of #N`, etc. out of a PR
//! body so the portal can mirror PR state to the linked spec issue's labels.
//!
//! Mirrors the regex shape of `.github/workflows/pr-spec-sync.yml` so that
//! removing the workflow once the portal is wired up changes nothing about
//! observed label behavior.

use std::collections::BTreeSet;

/// Extract every issue number referenced via the canonical link verbs.
/// Returns numbers in ascending order, deduplicated. Case-insensitive.
pub fn linked_issues(body: &str) -> Vec<u64> {
    let mut out = BTreeSet::new();
    for cap in RE_CLOSING.captures_iter(body) {
        if let Some(n) = cap.get(1).and_then(|m| m.as_str().parse().ok()) {
            out.insert(n);
        }
    }
    for cap in RE_REF.captures_iter(body) {
        if let Some(n) = cap.get(1).and_then(|m| m.as_str().parse().ok()) {
            out.insert(n);
        }
    }
    out.into_iter().collect()
}

// `lazy_static!` would be one option; a `OnceLock<Regex>` keeps the crate
// dependency-free and is plenty fast for the once-per-webhook hot path.
use std::sync::OnceLock;

static RE_CLOSING_INNER: OnceLock<regex::Regex> = OnceLock::new();
static RE_REF_INNER: OnceLock<regex::Regex> = OnceLock::new();

#[allow(non_upper_case_globals)]
static RE_CLOSING: LazyRegex = LazyRegex {
    cell: &RE_CLOSING_INNER,
    src: r"(?i)\b(?:close[sd]?|fix(?:e[sd])?|resolve[sd]?)\s+#(\d+)\b",
};

#[allow(non_upper_case_globals)]
static RE_REF: LazyRegex = LazyRegex {
    cell: &RE_REF_INNER,
    src: r"(?i)\b(?:part\s+of|refs|related)\s+#(\d+)\b",
};

struct LazyRegex {
    cell: &'static OnceLock<regex::Regex>,
    src: &'static str,
}

impl LazyRegex {
    fn captures_iter<'a>(&self, body: &'a str) -> regex::CaptureMatches<'_, 'a> {
        self.cell
            .get_or_init(|| regex::Regex::new(self.src).expect("regex compiles"))
            .captures_iter(body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_up_closes_and_part_of() {
        let body = "Implements the thing.\n\nCloses #58\nPart of #60";
        assert_eq!(linked_issues(body), vec![58, 60]);
    }

    #[test]
    fn dedupes_and_sorts() {
        let body = "Refs #99\nCloses #5\nPart of #5";
        assert_eq!(linked_issues(body), vec![5, 99]);
    }

    #[test]
    fn case_insensitive() {
        let body = "FIXES #1\ncloses #2\nResolves #3";
        assert_eq!(linked_issues(body), vec![1, 2, 3]);
    }

    #[test]
    fn ignores_bare_hash_refs() {
        // `#123` without a verb is a free-form reference, not a link.
        let body = "See #123 for context. Resolves #1";
        assert_eq!(linked_issues(body), vec![1]);
    }
}
