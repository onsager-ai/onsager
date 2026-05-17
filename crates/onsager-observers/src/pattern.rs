//! [`EventPattern`] — simple glob matching against `event_type` strings.
//!
//! Per the spec #361 notes: "Pattern matching is simple glob:
//! `artifact.*` matches `artifact.state_changed`, `artifact.sealed`,
//! etc. Full regex is not needed in v1."
//!
//! ## Supported shapes
//!
//! | Pattern             | Matches                                              |
//! |---------------------|------------------------------------------------------|
//! | `"*"`               | every event                                          |
//! | `"artifact.*"`      | every `event_type` starting with `"artifact."`       |
//! | `"node.completed"`  | exact match only                                     |
//!
//! No mid-string wildcards (`"a.*.b"`), no character classes, no
//! escapes — observers that need that can match the parsed
//! [`FactoryEvent`](onsager_spine::FactoryEvent) inside `on_event`.
//!
//! ## Why glob, not regex
//!
//! Event types are short dotted identifiers — a five-line prefix
//! matcher covers every real subscription pattern in 0.2. Reaching
//! for regex would add a dependency, a compile step, and (most
//! importantly) a way for an observer author to write a pattern that
//! silently doesn't match.

use serde::{Deserialize, Serialize};

/// A glob pattern against the wire `event_type` string.
///
/// Built via [`EventPattern::new`] or the [`From<&str>`] impl; match
/// with [`matches`](Self::matches).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EventPattern(String);

impl EventPattern {
    /// Construct a pattern from a string. The pattern is not
    /// validated up front; an unknown shape (e.g. `"a.*.b"`) will
    /// just never match anything.
    pub fn new(pattern: impl Into<String>) -> Self {
        Self(pattern.into())
    }

    /// The raw pattern string.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Returns `true` if `event_type` matches this pattern.
    pub fn matches(&self, event_type: &str) -> bool {
        match self.0.as_str() {
            // "*" — every event matches.
            "*" => true,
            // "<prefix>.*" — every event whose type starts with "<prefix>.".
            // The trailing dot in the comparison prevents "node.*" from
            // matching a hypothetical "nodemap" (which it shouldn't —
            // they're different namespaces).
            p if p.ends_with(".*") => {
                let prefix = &p[..p.len() - 1]; // strip just the "*", keep the "."
                event_type.starts_with(prefix)
            }
            // Exact match.
            p => p == event_type,
        }
    }
}

impl From<&str> for EventPattern {
    fn from(s: &str) -> Self {
        Self::new(s)
    }
}

impl From<String> for EventPattern {
    fn from(s: String) -> Self {
        Self::new(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn star_matches_everything() {
        let p = EventPattern::new("*");
        assert!(p.matches("artifact.state_changed"));
        assert!(p.matches("node.completed"));
        assert!(p.matches(""));
    }

    #[test]
    fn prefix_wildcard_matches_namespace() {
        let p = EventPattern::new("artifact.*");
        assert!(p.matches("artifact.state_changed"));
        assert!(p.matches("artifact.archived"));
        assert!(p.matches("artifact.registered"));
        // Multi-segment events still match — prefix is the whole rule.
        assert!(p.matches("artifact.some.nested"));
    }

    #[test]
    fn prefix_wildcard_respects_dot_boundary() {
        let p = EventPattern::new("node.*");
        // Same prefix, different namespace — must not match. This is
        // the key invariant: "node.*" must not eat "nodemap.foo".
        assert!(!p.matches("nodemap.touched"));
        // No dot at all — must not match.
        assert!(!p.matches("node"));
        // Real intent: matches any follow-on segment.
        assert!(p.matches("node.started"));
        assert!(p.matches("node.completed"));
    }

    #[test]
    fn exact_match() {
        let p = EventPattern::new("node.completed");
        assert!(p.matches("node.completed"));
        assert!(!p.matches("node.started"));
        assert!(!p.matches("node.completed.extra"));
    }

    #[test]
    fn empty_pattern_never_matches_real_events() {
        let p = EventPattern::new("");
        assert!(!p.matches("artifact.state_changed"));
        // An empty event_type matches the empty pattern — degenerate
        // but consistent with "exact match".
        assert!(p.matches(""));
    }

    #[test]
    fn roundtrip_serde() {
        let p = EventPattern::new("artifact.*");
        let s = serde_json::to_string(&p).unwrap();
        let back: EventPattern = serde_json::from_str(&s).unwrap();
        assert_eq!(p, back);
    }
}
