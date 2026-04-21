//! Namespace partitioning for the `events_ext` table.
//!
//! Each subsystem in the Onsager monorepo owns a namespace that scopes its
//! extension events. Adding a new component means adding a well-known constant
//! to [`Namespace`].

use std::fmt;

/// Error returned when a namespace string fails validation.
#[derive(Debug, thiserror::Error)]
pub enum NamespaceError {
    /// Length must be 1..=32 characters.
    #[error("invalid namespace length: {0} (must be 1..=32)")]
    InvalidLength(usize),

    /// Must start with a lowercase ASCII letter.
    #[error("namespace must start with a lowercase ASCII letter")]
    InvalidStart,

    /// Only `[a-z0-9_]` characters are allowed.
    #[error("invalid character in namespace: '{0}'")]
    InvalidChar(char),
}

/// A validated namespace that partitions the `events_ext` table between
/// components.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Namespace(String);

impl Namespace {
    /// Create a new namespace with validation.
    ///
    /// Rules:
    /// - 1..=32 characters
    /// - Must start with `[a-z]`
    /// - Only `[a-z0-9_]` allowed
    pub fn new(name: impl Into<String>) -> Result<Self, NamespaceError> {
        let name = name.into();
        let len = name.chars().count();
        if !(1..=32).contains(&len) {
            return Err(NamespaceError::InvalidLength(len));
        }
        let mut chars = name.chars();
        let first = chars.next().unwrap();
        if !first.is_ascii_lowercase() {
            return Err(NamespaceError::InvalidStart);
        }
        for ch in chars {
            if !ch.is_ascii_lowercase() && !ch.is_ascii_digit() && ch != '_' {
                return Err(NamespaceError::InvalidChar(ch));
            }
        }
        Ok(Self(name))
    }

    /// Return the namespace as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    // -- Well-known constants ---------------------------------------------------
    // These are the soft contract for namespace ownership across the monorepo.
    // Adding a new component means adding a constant here.

    /// Namespace for the forge component.
    pub fn forge() -> Self {
        Self::new("forge").unwrap()
    }

    /// Namespace for the stiglab component.
    pub fn stiglab() -> Self {
        Self::new("stiglab").unwrap()
    }

    /// Namespace for the synodic component.
    pub fn synodic() -> Self {
        Self::new("synodic").unwrap()
    }

    /// Namespace for the ising component.
    pub fn ising() -> Self {
        Self::new("ising").unwrap()
    }

    /// Namespace for the telegramable component.
    pub fn telegramable() -> Self {
        Self::new("telegramable").unwrap()
    }

    /// Namespace for workflow-runtime events (issue #80). Owned by forge
    /// but consumed by the dashboard and stiglab too, so it gets its own
    /// namespace instead of piggybacking on `forge:`.
    pub fn workflow() -> Self {
        Self::new("workflow").unwrap()
    }
}

impl fmt::Display for Namespace {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_namespaces() {
        assert!(Namespace::new("stiglab").is_ok());
        assert!(Namespace::new("a").is_ok());
        assert!(Namespace::new("my_ns_2").is_ok());
        assert!(Namespace::new("a1234567890123456789012345678901").is_ok()); // 32 chars
    }

    #[test]
    fn invalid_length() {
        assert!(matches!(
            Namespace::new(""),
            Err(NamespaceError::InvalidLength(0))
        ));
        let long = "a".repeat(33);
        assert!(matches!(
            Namespace::new(long),
            Err(NamespaceError::InvalidLength(33))
        ));
    }

    #[test]
    fn invalid_start() {
        assert!(matches!(
            Namespace::new("1abc"),
            Err(NamespaceError::InvalidStart)
        ));
        assert!(matches!(
            Namespace::new("_abc"),
            Err(NamespaceError::InvalidStart)
        ));
        assert!(matches!(
            Namespace::new("Abc"),
            Err(NamespaceError::InvalidStart)
        ));
    }

    #[test]
    fn invalid_char() {
        assert!(matches!(
            Namespace::new("ab-cd"),
            Err(NamespaceError::InvalidChar('-'))
        ));
        assert!(matches!(
            Namespace::new("ab.cd"),
            Err(NamespaceError::InvalidChar('.'))
        ));
        assert!(matches!(
            Namespace::new("abCd"),
            Err(NamespaceError::InvalidChar('C'))
        ));
    }

    #[test]
    fn well_known_constants() {
        assert_eq!(Namespace::forge().as_str(), "forge");
        assert_eq!(Namespace::stiglab().as_str(), "stiglab");
        assert_eq!(Namespace::synodic().as_str(), "synodic");
        assert_eq!(Namespace::ising().as_str(), "ising");
        assert_eq!(Namespace::telegramable().as_str(), "telegramable");
    }

    #[test]
    fn display() {
        assert_eq!(Namespace::stiglab().to_string(), "stiglab");
    }
}
