//! Bundle model and Warehouse trait.
//!
//! See `specs/warehouse-and-delivery-v0.1.md` §4.2 and §8. A **bundle** is an
//! immutable, content-addressed snapshot of what Forge produced at one release.
//! The [`Warehouse`] trait abstracts over storage backends (filesystem,
//! Postgres large objects, S3); v0.1 ships the filesystem backend here.
//!
//! Invariants (warehouse-and-delivery-v0.1 §9):
//! 1. Once a bundle is sealed, no field — including its content blobs — may
//!    change. Re-sealing the same inputs is a no-op at the blob layer and is
//!    rejected at the bundle row layer (UNIQUE on `artifact_id, version`).
//! 2. Per-artifact `version` is monotonic and gap-free.
//! 3. `supersedes` forms a linear chain.
//!
//! The trait is deliberately small: one sealing entry point, plus `fetch` /
//! `exists` for introspection. Retention, redaction, and composition are
//! out of scope for v0.1 (§11).
//!
//! Sealing failure modes:
//! - Content-addressing means orphan blobs from a half-finished seal cost only
//!   disk space; since the bundle row never commits, they are unreachable and
//!   can be garbage-collected later.
//! - The `(artifact_id, version)` unique constraint catches concurrent seals
//!   and races with a [`SealError::VersionConflict`].

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use chrono::{DateTime, SubsecRound, Utc};
use onsager_artifact::{ArtifactId, BundleId};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::PgPool;

// ---------------------------------------------------------------------------
// BundleId hashing helper
// ---------------------------------------------------------------------------

/// Derive a deterministic [`BundleId`] from the canonical manifest, artifact
/// id, and version. Two seals of the same artifact at the same version with
/// identical manifests produce the same id (the `(artifact_id, version)`
/// UNIQUE then rejects the reseal as `VersionConflict`); two different
/// artifacts with identical files produce different ids, so they do not
/// collide.
///
/// The [`BundleId`] type itself lives in `onsager-artifact`; only the hashing
/// rule (which depends on [`Manifest`]) lives here.
fn bundle_id_from_manifest(
    manifest: &Manifest,
    artifact_id: &ArtifactId,
    version: u32,
) -> BundleId {
    // Hash over (artifact_id, version, canonical manifest bytes). Including
    // artifact_id + version ensures two artifacts that happen to ship
    // identical files don't collide on the same BundleId; identical reseals
    // of the same artifact at the same version do collide (which the
    // (artifact_id, version) UNIQUE then rejects as VersionConflict).
    let mut hasher = Sha256::new();
    hasher.update(artifact_id.as_str().as_bytes());
    hasher.update([0u8]);
    hasher.update(version.to_be_bytes());
    hasher.update([0u8]);
    let canonical = serde_json::to_vec(manifest).expect("Manifest must be serialisable as JSON");
    hasher.update(&canonical);
    BundleId::new(format!("bnd_{}", hex::encode(hasher.finalize())))
}

// ---------------------------------------------------------------------------
// Manifest
// ---------------------------------------------------------------------------

/// One entry in a bundle manifest — a file path plus its content hash and size.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestEntry {
    pub path: String,
    pub size: u64,
    /// Hex SHA-256 of the blob content.
    pub content_hash: String,
}

/// Ordered list of manifest entries.
///
/// Sorting by `path` is part of the canonicalisation that makes [`BundleId`]
/// deterministic across reseals.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub entries: Vec<ManifestEntry>,
}

impl Manifest {
    pub fn total_size(&self) -> u64 {
        self.entries.iter().map(|e| e.size).sum()
    }
}

// ---------------------------------------------------------------------------
// Outputs (input to sealing)
// ---------------------------------------------------------------------------

/// The raw bytes Forge hands to the warehouse on release. Each entry is
/// `(relative_path, bytes)`.
#[derive(Debug, Clone, Default)]
pub struct Outputs {
    pub files: Vec<(String, Vec<u8>)>,
}

impl Outputs {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, path: impl Into<String>, bytes: impl Into<Vec<u8>>) {
        self.files.push((path.into(), bytes.into()));
    }
}

// ---------------------------------------------------------------------------
// Bundle (the sealed record)
// ---------------------------------------------------------------------------

/// A sealed, immutable bundle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bundle {
    pub bundle_id: BundleId,
    pub artifact_id: ArtifactId,
    pub version: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supersedes: Option<BundleId>,
    pub manifest: Manifest,
    /// Backend-specific URI pointing at the sealed content root
    /// (e.g. `file:///var/onsager/blobs` for the filesystem backend).
    pub content_ref: String,
    pub sealed_at: DateTime<Utc>,
    pub sealed_by: String,
    pub metadata: serde_json::Value,
}

// ---------------------------------------------------------------------------
// Warehouse trait
// ---------------------------------------------------------------------------

/// Request to seal a new bundle.
#[derive(Debug, Clone)]
pub struct SealRequest {
    pub artifact_id: ArtifactId,
    /// The session that produced these outputs (stiglab session id, or
    /// an operator/system name for non-session seals).
    pub sealed_by: String,
    /// Caller-provided metadata merged into `bundle.metadata`.
    pub metadata: serde_json::Value,
    pub outputs: Outputs,
}

/// Storage backend for sealed bundles (§8).
#[async_trait]
pub trait Warehouse: Send + Sync {
    /// Seal `outputs` as a new bundle for `artifact_id`. Allocates the next
    /// `version` for the artifact and links `supersedes` to the prior bundle,
    /// if any.
    async fn seal(&self, request: SealRequest) -> Result<Bundle, SealError>;

    /// Look up a sealed bundle by id.
    async fn fetch(&self, bundle_id: &BundleId) -> Result<Bundle, FetchError>;

    /// Whether a bundle with the given id exists.
    async fn exists(&self, bundle_id: &BundleId) -> Result<bool, FetchError>;
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum SealError {
    #[error("warehouse I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("warehouse database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("artifact {0} not found")]
    ArtifactNotFound(ArtifactId),
    #[error("version conflict for artifact {artifact_id}: version {version} already sealed")]
    VersionConflict {
        artifact_id: ArtifactId,
        version: u32,
    },
    #[error("invalid seal request: {0}")]
    Invalid(String),
}

#[derive(Debug, thiserror::Error)]
pub enum FetchError {
    #[error("bundle {0} not found")]
    NotFound(BundleId),
    #[error("warehouse I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("warehouse database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("manifest corrupted: {0}")]
    Corrupted(String),
}

// ---------------------------------------------------------------------------
// Filesystem backend
// ---------------------------------------------------------------------------

/// Filesystem warehouse: content blobs live on disk under `root/blobs/`,
/// keyed by their SHA-256, and the manifest is persisted to Postgres.
///
/// Default for local dev per spec §8.
pub struct FilesystemWarehouse {
    pool: PgPool,
    root: PathBuf,
}

impl FilesystemWarehouse {
    pub fn new(pool: PgPool, root: impl Into<PathBuf>) -> Self {
        Self {
            pool,
            root: root.into(),
        }
    }

    fn blobs_dir(&self) -> PathBuf {
        self.root.join("blobs")
    }

    fn blob_path(&self, hash: &str) -> PathBuf {
        // Fan out by the first two hex chars to keep any single directory
        // manageable under large bundle counts.
        self.blobs_dir().join(&hash[..2]).join(hash)
    }

    fn content_ref_uri(&self) -> String {
        format!("file://{}", self.blobs_dir().display())
    }

    async fn write_blob(&self, hash: &str, bytes: &[u8]) -> Result<(), std::io::Error> {
        let path = self.blob_path(hash);
        if tokio::fs::try_exists(&path).await? {
            return Ok(());
        }
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        // Write to a temp file alongside, then rename — atomic on POSIX for
        // same-directory renames. Content-addressing means a crash between
        // writes produces orphan temps, not corrupt blobs.
        let tmp = path.with_extension("tmp");
        tokio::fs::write(&tmp, bytes).await?;
        tokio::fs::rename(&tmp, &path).await?;
        Ok(())
    }

    async fn read_blob(&self, hash: &str) -> Result<Vec<u8>, std::io::Error> {
        tokio::fs::read(self.blob_path(hash)).await
    }
}

/// Returns the hex SHA-256 of `bytes`.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

fn build_manifest(outputs: &Outputs) -> Result<Manifest, SealError> {
    let mut seen = std::collections::HashSet::new();
    for (path, _) in &outputs.files {
        if !seen.insert(path.as_str()) {
            return Err(SealError::Invalid(format!(
                "duplicate path in outputs: {path}"
            )));
        }
    }
    let mut entries: Vec<ManifestEntry> = outputs
        .files
        .iter()
        .map(|(path, bytes)| ManifestEntry {
            path: path.clone(),
            size: bytes.len() as u64,
            content_hash: sha256_hex(bytes),
        })
        .collect();
    entries.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(Manifest { entries })
}

#[async_trait]
impl Warehouse for FilesystemWarehouse {
    async fn seal(&self, request: SealRequest) -> Result<Bundle, SealError> {
        let SealRequest {
            artifact_id,
            sealed_by,
            metadata,
            outputs,
        } = request;

        // 1. Build the manifest (sorted, hashed). Also index the raw bytes by
        //    path so blob writes align with the sorted manifest entries —
        //    zipping `outputs.files` (insertion order) with `manifest.entries`
        //    (sorted) would write the wrong blob under each content hash.
        let manifest = build_manifest(&outputs)?;
        let bytes_by_path: std::collections::HashMap<&str, &[u8]> = outputs
            .files
            .iter()
            .map(|(path, bytes)| (path.as_str(), bytes.as_slice()))
            .collect();

        // 2. Determine the next version and prior bundle by scanning existing
        //    bundles for this artifact. Done inside a transaction to guard
        //    against concurrent seals.
        let mut tx = self.pool.begin().await?;

        // Confirm the artifact exists; FKs would catch this too but a clear
        // error message is worth the extra round-trip.
        let exists: Option<(String,)> =
            sqlx::query_as("SELECT artifact_id FROM artifacts WHERE artifact_id = $1")
                .bind(artifact_id.as_str())
                .fetch_optional(&mut *tx)
                .await?;
        if exists.is_none() {
            return Err(SealError::ArtifactNotFound(artifact_id));
        }

        let row: Option<(i32, String)> = sqlx::query_as(
            "SELECT version, bundle_id FROM bundles \
             WHERE artifact_id = $1 ORDER BY version DESC LIMIT 1",
        )
        .bind(artifact_id.as_str())
        .fetch_optional(&mut *tx)
        .await?;

        let (prior_version, supersedes) = match row {
            Some((v, id)) => (v as u32, Some(BundleId::new(id))),
            None => (0u32, None),
        };
        let version = prior_version + 1;
        let bundle_id = bundle_id_from_manifest(&manifest, &artifact_id, version);

        // 3. Write blobs to disk. Content-addressed — safe to retry.
        for entry in &manifest.entries {
            let bytes = bytes_by_path
                .get(entry.path.as_str())
                .expect("manifest entries derive from outputs.files keys");
            self.write_blob(&entry.content_hash, bytes).await?;
        }

        let content_ref = self.content_ref_uri();
        // Postgres TIMESTAMPTZ is microsecond-precision; truncate here so the
        // Bundle we return round-trips byte-for-byte with a subsequent fetch.
        let sealed_at = Utc::now().trunc_subsecs(6);
        let manifest_json = serde_json::to_value(&manifest)
            .map_err(|e| SealError::Invalid(format!("manifest serialisation: {e}")))?;

        // 4. Insert the bundle row. A UNIQUE(artifact_id, version) collision
        //    here is the "someone else sealed first" race.
        let insert = sqlx::query(
            "INSERT INTO bundles \
             (bundle_id, artifact_id, version, supersedes, manifest, content_ref, sealed_at, sealed_by, metadata) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
        )
        .bind(bundle_id.as_str())
        .bind(artifact_id.as_str())
        .bind(version as i32)
        .bind(supersedes.as_ref().map(|b| b.as_str()))
        .bind(&manifest_json)
        .bind(&content_ref)
        .bind(sealed_at)
        .bind(&sealed_by)
        .bind(&metadata)
        .execute(&mut *tx)
        .await;

        match insert {
            Err(sqlx::Error::Database(dbe)) if dbe.is_unique_violation() => {
                return Err(SealError::VersionConflict {
                    artifact_id,
                    version,
                });
            }
            Err(e) => return Err(SealError::Database(e)),
            Ok(_) => {}
        }

        tx.commit().await?;

        Ok(Bundle {
            bundle_id,
            artifact_id,
            version,
            supersedes,
            manifest,
            content_ref,
            sealed_at,
            sealed_by,
            metadata,
        })
    }

    async fn fetch(&self, bundle_id: &BundleId) -> Result<Bundle, FetchError> {
        let row: Option<(
            String,
            String,
            i32,
            Option<String>,
            serde_json::Value,
            String,
            DateTime<Utc>,
            String,
            serde_json::Value,
        )> = sqlx::query_as(
            "SELECT bundle_id, artifact_id, version, supersedes, manifest, content_ref, sealed_at, sealed_by, metadata \
             FROM bundles WHERE bundle_id = $1",
        )
        .bind(bundle_id.as_str())
        .fetch_optional(&self.pool)
        .await?;

        let (bid, aid, version, supersedes, manifest, content_ref, sealed_at, sealed_by, metadata) =
            row.ok_or_else(|| FetchError::NotFound(bundle_id.clone()))?;

        let manifest: Manifest = serde_json::from_value(manifest)
            .map_err(|e| FetchError::Corrupted(format!("manifest JSON: {e}")))?;

        Ok(Bundle {
            bundle_id: BundleId::new(bid),
            artifact_id: ArtifactId::new(aid),
            version: version as u32,
            supersedes: supersedes.map(BundleId::new),
            manifest,
            content_ref,
            sealed_at,
            sealed_by,
            metadata,
        })
    }

    async fn exists(&self, bundle_id: &BundleId) -> Result<bool, FetchError> {
        let row: Option<(String,)> =
            sqlx::query_as("SELECT bundle_id FROM bundles WHERE bundle_id = $1")
                .bind(bundle_id.as_str())
                .fetch_optional(&self.pool)
                .await?;
        Ok(row.is_some())
    }
}

impl FilesystemWarehouse {
    /// Read a blob by its content hash. Useful for consumers that want to
    /// stream a bundle's contents without re-sealing it.
    ///
    /// Rejects malformed hashes (wrong length or non-hex) with
    /// [`FetchError::Corrupted`] rather than panicking in the path builder.
    pub async fn read_blob_by_hash(&self, hash: &str) -> Result<Vec<u8>, FetchError> {
        if hash.len() != 64 || !hash.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(FetchError::Corrupted(format!(
                "malformed blob hash: {hash}"
            )));
        }
        match self.read_blob(hash).await {
            Ok(bytes) => Ok(bytes),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                Err(FetchError::Corrupted(format!("missing blob: {hash}")))
            }
            Err(e) => Err(FetchError::Io(e)),
        }
    }

    /// Exposes the blobs directory (primarily for testing).
    pub fn root(&self) -> &Path {
        &self.root
    }
}

// ---------------------------------------------------------------------------
// Tests (unit — no DB)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_matches_known_vector() {
        // SHA-256("abc") = ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn manifest_entries_sorted_by_path() {
        let mut outputs = Outputs::new();
        outputs.push("z.txt", b"z".to_vec());
        outputs.push("a.txt", b"a".to_vec());
        outputs.push("m.txt", b"m".to_vec());

        let manifest = build_manifest(&outputs).unwrap();
        let paths: Vec<_> = manifest.entries.iter().map(|e| e.path.as_str()).collect();
        assert_eq!(paths, vec!["a.txt", "m.txt", "z.txt"]);
    }

    #[test]
    fn manifest_entry_hash_matches_bytes_at_that_path() {
        // Regression guard: insertion order (z, a) differs from sorted order
        // (a, z). Each entry's content_hash must be the hash of the bytes
        // at its path, not a blindly-zipped slot.
        let mut outputs = Outputs::new();
        outputs.push("z.txt", b"z-bytes".to_vec());
        outputs.push("a.txt", b"a-bytes".to_vec());

        let manifest = build_manifest(&outputs).unwrap();
        let a_entry = manifest.entries.iter().find(|e| e.path == "a.txt").unwrap();
        let z_entry = manifest.entries.iter().find(|e| e.path == "z.txt").unwrap();
        assert_eq!(a_entry.content_hash, sha256_hex(b"a-bytes"));
        assert_eq!(z_entry.content_hash, sha256_hex(b"z-bytes"));
    }

    #[test]
    fn manifest_rejects_duplicate_paths() {
        let mut outputs = Outputs::new();
        outputs.push("a.txt", b"first".to_vec());
        outputs.push("a.txt", b"second".to_vec());

        let err = build_manifest(&outputs).unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("duplicate"), "unexpected: {msg}");
    }

    #[test]
    fn bundle_id_deterministic_for_same_inputs() {
        let manifest = Manifest {
            entries: vec![ManifestEntry {
                path: "a.txt".into(),
                size: 1,
                content_hash: "deadbeef".into(),
            }],
        };
        let artifact_id = ArtifactId::new("art_test");
        let a = bundle_id_from_manifest(&manifest, &artifact_id, 1);
        let b = bundle_id_from_manifest(&manifest, &artifact_id, 1);
        assert_eq!(a, b);

        // Different version => different id.
        let c = bundle_id_from_manifest(&manifest, &artifact_id, 2);
        assert_ne!(a, c);

        // Different artifact id => different id.
        let d = bundle_id_from_manifest(&manifest, &ArtifactId::new("art_other"), 1);
        assert_ne!(a, d);
    }

    #[test]
    fn bundle_id_format() {
        let manifest = Manifest { entries: vec![] };
        let id = bundle_id_from_manifest(&manifest, &ArtifactId::new("art_x"), 1);
        assert!(id.as_str().starts_with("bnd_"));
        assert_eq!(id.as_str().len(), 68); // "bnd_" + 64 hex chars
    }

    #[test]
    fn manifest_total_size() {
        let manifest = Manifest {
            entries: vec![
                ManifestEntry {
                    path: "a".into(),
                    size: 10,
                    content_hash: "x".into(),
                },
                ManifestEntry {
                    path: "b".into(),
                    size: 5,
                    content_hash: "y".into(),
                },
            ],
        };
        assert_eq!(manifest.total_size(), 15);
    }

    #[test]
    fn bundle_roundtrip_json() {
        let bundle = Bundle {
            bundle_id: BundleId::new("bnd_abc"),
            artifact_id: ArtifactId::new("art_xyz"),
            version: 3,
            supersedes: Some(BundleId::new("bnd_prev")),
            manifest: Manifest {
                entries: vec![ManifestEntry {
                    path: "README.md".into(),
                    size: 128,
                    content_hash: "cafe".into(),
                }],
            },
            content_ref: "file:///tmp/warehouse".into(),
            sealed_at: Utc::now(),
            sealed_by: "sess_01".into(),
            metadata: serde_json::json!({"kind": "code"}),
        };
        let json = serde_json::to_value(&bundle).unwrap();
        let roundtrip: Bundle = serde_json::from_value(json).unwrap();
        assert_eq!(roundtrip, bundle);
    }

    #[test]
    fn seal_error_display() {
        let err = SealError::VersionConflict {
            artifact_id: ArtifactId::new("art_a"),
            version: 4,
        };
        let msg = format!("{err}");
        assert!(msg.contains("art_a"));
        assert!(msg.contains("4"));
    }
}
