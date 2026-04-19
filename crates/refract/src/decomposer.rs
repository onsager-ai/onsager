//! Decomposer trait and the hard-coded `file_migration` implementation.

use std::collections::HashMap;

use onsager_artifact::{Artifact, ArtifactId, Kind};
use thiserror::Error;

use crate::intent::Intent;

/// Output of a successful decomposition — the per-file artifacts a
/// decomposer synthesized. Callers are responsible for registering them
/// with the factory (e.g. via Forge's `POST /api/artifacts` endpoint or
/// a direct `artifact.registered` spine emission).
#[derive(Debug)]
pub struct DecompositionResult {
    pub artifacts: Vec<Artifact>,
}

impl DecompositionResult {
    pub fn artifact_ids(&self) -> Vec<ArtifactId> {
        self.artifacts
            .iter()
            .map(|a| a.artifact_id.clone())
            .collect()
    }
}

#[derive(Debug, Error)]
pub enum DecomposerError {
    #[error("decomposer '{0}' not registered")]
    NotRegistered(String),
    #[error("decomposer failed: {0}")]
    Failed(String),
}

/// A Refract decomposer expands an intent into artifacts.
pub trait Decomposer: Send + Sync {
    /// Stable identifier matched against `Intent::intent_class`.
    fn name(&self) -> &str;

    fn decompose(&self, intent: &Intent) -> Result<DecompositionResult, DecomposerError>;
}

/// Registry of decomposers keyed by `Decomposer::name()`. Explicit
/// registration (rather than inventory / ctor tricks) so the set of
/// installed decomposers is auditable at a glance.
#[derive(Default)]
pub struct DecomposerRegistry {
    items: HashMap<String, Box<dyn Decomposer>>,
}

impl DecomposerRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register<D: Decomposer + 'static>(&mut self, decomposer: D) {
        self.items
            .insert(decomposer.name().to_string(), Box::new(decomposer));
    }

    pub fn decompose(&self, intent: &Intent) -> Result<DecompositionResult, DecomposerError> {
        let Some(d) = self.items.get(&intent.intent_class) else {
            return Err(DecomposerError::NotRegistered(intent.intent_class.clone()));
        };
        d.decompose(intent)
    }

    pub fn names(&self) -> Vec<&str> {
        self.items.keys().map(String::as_str).collect()
    }
}

/// Hard-coded decomposer for `"file_migration"` intents (issue #35 MVP).
///
/// Reads a `"files"` array from the intent payload; synthesizes one
/// `Kind::Code` artifact per path. The produced artifacts are owned by the
/// original submitter and named after the file.
///
/// Payload shape:
/// ```json
/// { "files": ["src/auth.rs", "src/db.rs"] }
/// ```
pub struct FileMigrationDecomposer;

impl FileMigrationDecomposer {
    pub const NAME: &'static str = "file_migration";
}

impl Decomposer for FileMigrationDecomposer {
    fn name(&self) -> &str {
        Self::NAME
    }

    fn decompose(&self, intent: &Intent) -> Result<DecompositionResult, DecomposerError> {
        let files = intent
            .payload
            .get("files")
            .and_then(|v| v.as_array())
            .ok_or_else(|| {
                DecomposerError::Failed(
                    "file_migration payload must contain a 'files' array".into(),
                )
            })?;

        if files.is_empty() {
            return Err(DecomposerError::Failed(
                "file_migration payload 'files' array is empty".into(),
            ));
        }

        let mut artifacts = Vec::with_capacity(files.len());
        for (idx, path_val) in files.iter().enumerate() {
            let path = path_val
                .as_str()
                .ok_or_else(|| DecomposerError::Failed(format!("files[{idx}] is not a string")))?;
            let name = format!("{} ({path})", intent.description);
            artifacts.push(Artifact::new(
                Kind::Code,
                name,
                intent.submitter.clone(),
                "refract",
                vec![],
            ));
        }

        Ok(DecompositionResult { artifacts })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn mk_intent(class: &str, payload: serde_json::Value) -> Intent {
        Intent::new(class, "test", "marvin", payload)
    }

    #[test]
    fn file_migration_produces_one_artifact_per_file() {
        let d = FileMigrationDecomposer;
        let intent = mk_intent(
            FileMigrationDecomposer::NAME,
            json!({ "files": ["src/a.rs", "src/b.rs", "src/c.rs"] }),
        );
        let result = d.decompose(&intent).expect("decomposes");
        assert_eq!(result.artifacts.len(), 3);
        for a in &result.artifacts {
            assert_eq!(a.kind, Kind::Code);
            assert_eq!(a.owner, "marvin");
        }
    }

    #[test]
    fn file_migration_rejects_missing_files_field() {
        let d = FileMigrationDecomposer;
        let intent = mk_intent(FileMigrationDecomposer::NAME, json!({}));
        assert!(matches!(
            d.decompose(&intent),
            Err(DecomposerError::Failed(_))
        ));
    }

    #[test]
    fn file_migration_rejects_empty_files_array() {
        let d = FileMigrationDecomposer;
        let intent = mk_intent(FileMigrationDecomposer::NAME, json!({ "files": [] }));
        assert!(matches!(
            d.decompose(&intent),
            Err(DecomposerError::Failed(_))
        ));
    }

    #[test]
    fn file_migration_rejects_non_string_entries() {
        let d = FileMigrationDecomposer;
        let intent = mk_intent(
            FileMigrationDecomposer::NAME,
            json!({ "files": ["ok.rs", 42] }),
        );
        assert!(matches!(
            d.decompose(&intent),
            Err(DecomposerError::Failed(_))
        ));
    }

    #[test]
    fn registry_dispatches_by_intent_class() {
        let mut registry = DecomposerRegistry::new();
        registry.register(FileMigrationDecomposer);
        let intent = mk_intent(FileMigrationDecomposer::NAME, json!({ "files": ["x.rs"] }));
        let result = registry.decompose(&intent).expect("registered");
        assert_eq!(result.artifacts.len(), 1);
    }

    #[test]
    fn registry_errors_on_unknown_intent_class() {
        let registry = DecomposerRegistry::new();
        let intent = mk_intent("unknown_class", json!({}));
        assert!(matches!(
            registry.decompose(&intent),
            Err(DecomposerError::NotRegistered(_))
        ));
    }
}
