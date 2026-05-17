//! Script executor — spawns a subprocess, captures stdout, maps exit
//! code to success / failure.
//!
//! The simplest non-trivial executor in the catalog: deterministic by
//! construction. It is the reference implementation other executors
//! are measured against, and the verification surface for the
//! executor-dispatch chain end-to-end (see #354 / EXE-02).
//!
//! # Provenance
//!
//! Declared provenance is always `Deterministic { source: Script }`,
//! regardless of inputs. The kernel's invariant 2 (ADR 0018) still
//! applies — a Script node consuming an `Uncertain` input emits
//! `Uncertain` per the max-uncertainty rule — but the executor itself
//! does not upgrade or downgrade its declared output. Only Verify
//! (EXE-04) is allowed to upgrade `Uncertain` upstream into
//! `Deterministic` downstream.
//!
//! # Sandboxing
//!
//! No sandboxing in v1 — the subprocess inherits its parent's view of
//! the filesystem and network, modulo the `env` map (passed verbatim)
//! and a cleared environment otherwise. Sandboxing (seccomp,
//! namespaces, cgroup limits) is a follow-up. The `timeout_secs` field
//! is the only hard kill-switch available today.
//!
//! # Output shape
//!
//! On success the executor produces a single `Artifact` whose latest
//! version's `content_ref` carries the captured stdout inline via an
//! `inline:` URI scheme (base64-encoded, byte-safe). The convention is
//! local to this module today; future executors that produce inline
//! content reuse the same scheme, and a real warehouse-backed scheme
//! will replace it once content storage is wired up.

use std::collections::HashMap;
use std::process::Stdio;
use std::time::Duration;

use async_trait::async_trait;
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use chrono::Utc;
use onsager_artifact::{Artifact, ArtifactVersion, ContentRef, Kind, Provenance, SourceTag};
use onsager_substrate::executor::Executor as SubstrateExecutor;
use serde::{Deserialize, Serialize};
use tokio::process::Command;

use crate::context::{ExecutorContext, ExecutorOutputs};
use crate::error::ExecutorError;
use crate::executor::Executor;

/// The URI scheme prefix used to encode inline content on a Script
/// executor's output artifact. Public so downstream consumers can
/// detect / decode without reimplementing the convention.
pub const INLINE_URI_PREFIX: &str = "inline:base64,";

/// Decode an inline-scheme URI back to the raw bytes the executor
/// captured. Returns `None` if `uri` is not an inline URI or if its
/// payload is not valid base64.
pub fn decode_inline_body(uri: &str) -> Option<Vec<u8>> {
    let payload = uri.strip_prefix(INLINE_URI_PREFIX)?;
    BASE64.decode(payload).ok()
}

/// Configuration for a Script-executor node.
///
/// Per-instance: each Script node in a workflow carries its own
/// `command` / `env` / `timeout_secs`. The runtime trait surface
/// ([`Executor`]) is implemented on this struct directly, so the same
/// type is both the wire-format config and the runtime behavior.
///
/// Wiring per-node-stateful executors through the registry-backed
/// [`crate::dispatch`] path is a follow-up (RUN-01, #359). Today the
/// scheduler can build a `ScriptExecutor` instance and invoke
/// `execute` on it directly.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScriptExecutor {
    /// Argv: `command[0]` is the program, the rest are arguments.
    /// Empty argv is rejected at execute time with
    /// [`ExecutorError::Failed`].
    pub command: Vec<String>,
    /// Environment variables to set for the subprocess. The
    /// subprocess starts with an otherwise-cleared environment —
    /// nothing from the parent leaks in by default.
    pub env: HashMap<String, String>,
    /// Wall-clock timeout. Exceeded → child is killed and the
    /// executor returns [`ExecutorError::Failed`].
    pub timeout_secs: u64,
}

impl ScriptExecutor {
    /// Build an executor from an argv. Default timeout is 60s, no env.
    pub fn new<I, S>(command: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            command: command.into_iter().map(Into::into).collect(),
            env: HashMap::new(),
            timeout_secs: 60,
        }
    }

    /// Set the timeout. Builder-style for ergonomic construction in
    /// tests and call sites.
    pub fn with_timeout_secs(mut self, secs: u64) -> Self {
        self.timeout_secs = secs;
        self
    }

    /// Set the env. Builder-style.
    pub fn with_env(mut self, env: HashMap<String, String>) -> Self {
        self.env = env;
        self
    }
}

// ---------------------------------------------------------------------------
// Substrate side (sync, serializable) — what nodes carry on the wire.
//
// Matches the Agent / Verify shape: the same struct implements both the
// substrate trait (typetag round-trip + kernel invariant checks) and
// the runtime trait (async execute). The kernel discriminates Script
// from non-Script via `executor_kind() == "script"`; provenance is
// always `Deterministic { source: Script }` regardless of inputs,
// with invariant 2 promoting it to the max-uncertainty of the inputs
// at validate time per ADR 0018.
// ---------------------------------------------------------------------------

#[typetag::serde(name = "script")]
impl SubstrateExecutor for ScriptExecutor {
    fn executor_kind(&self) -> &'static str {
        "script"
    }

    fn declared_provenance(&self, _inputs: &[Provenance]) -> Provenance {
        Provenance::Deterministic {
            source: SourceTag::Script,
        }
    }
}

#[async_trait]
impl Executor for ScriptExecutor {
    fn executor_kind(&self) -> &'static str {
        "script"
    }

    fn declared_provenance(&self, _inputs: &[Provenance]) -> Provenance {
        Provenance::Deterministic {
            source: SourceTag::Script,
        }
    }

    async fn execute(&self, ctx: ExecutorContext) -> Result<ExecutorOutputs, ExecutorError> {
        let (program, args) = self
            .command
            .split_first()
            .ok_or_else(|| ExecutorError::failed("script command is empty"))?;

        let mut cmd = Command::new(program);
        cmd.args(args);
        cmd.env_clear();
        cmd.envs(&self.env);
        cmd.stdin(Stdio::null());
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());
        cmd.kill_on_drop(true);

        let timeout = Duration::from_secs(self.timeout_secs);
        let output = tokio::time::timeout(timeout, cmd.output())
            .await
            .map_err(|_| {
                ExecutorError::failed(format!("script timed out after {}s", self.timeout_secs))
            })?
            .map_err(|e| ExecutorError::failed(format!("script spawn failed: {e}")))?;

        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();

        if !output.status.success() {
            // Failure path: surface exit code + stderr to the caller.
            let exit_code = output.status.code().unwrap_or(-1);
            return Err(ExecutorError::ScriptFailed { exit_code, stderr });
        }

        // Success path: log stderr (it may be non-empty even on
        // success — e.g. progress chatter) and produce an artifact.
        if !stderr.is_empty() {
            tracing::debug!(node_id = %ctx.node_id, stderr = %stderr, "script stderr");
        }

        let artifact = build_output_artifact(&ctx, &self.command, &output.stdout);
        let id = artifact.artifact_id.clone();
        Ok(ExecutorOutputs::single(id, artifact))
    }
}

/// Build the success-path output artifact. Encapsulated so the
/// inline-content convention has a single owner (and a single place to
/// migrate when warehouse-backed storage lands).
fn build_output_artifact(ctx: &ExecutorContext, command: &[String], stdout: &[u8]) -> Artifact {
    let mut artifact = Artifact::new(
        Kind::Document,
        "script-output",
        "onsager-nodes",
        "script-executor",
        vec![],
    );
    let uri = format!("{INLINE_URI_PREFIX}{}", BASE64.encode(stdout));
    artifact.versions.push(ArtifactVersion {
        version: 1,
        created_at: Utc::now(),
        created_by_session: format!("node:{}", ctx.node_id),
        content_ref: ContentRef {
            uri,
            checksum: None,
        },
        change_summary: format!("script: {}", command.join(" ")),
        parent_version: None,
    });
    artifact.current_version = 1;
    artifact.provenance = Provenance::Deterministic {
        source: SourceTag::Script,
    };
    artifact.produced_by_node = Some(ctx.node_id);
    artifact
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spine::test_support::MockSpine;
    use onsager_artifact::NodeId;
    use std::sync::Arc;

    fn ctx() -> ExecutorContext {
        ExecutorContext {
            node_id: NodeId::generate(),
            inputs: vec![],
            spine: Arc::new(MockSpine::default()),
        }
    }

    #[test]
    fn executor_kind_is_script() {
        let exec = ScriptExecutor::new(["true"]);
        assert_eq!(Executor::executor_kind(&exec), "script");
        assert_eq!(SubstrateExecutor::executor_kind(&exec), "script");
    }

    #[test]
    fn declared_provenance_is_deterministic_script_with_no_inputs() {
        let exec = ScriptExecutor::new(["true"]);
        assert_eq!(
            Executor::declared_provenance(&exec, &[]),
            Provenance::Deterministic {
                source: SourceTag::Script,
            }
        );
        // Substrate side must agree — the kernel invariant checks
        // and the runtime executor share the same provenance contract.
        assert_eq!(
            SubstrateExecutor::declared_provenance(&exec, &[]),
            Executor::declared_provenance(&exec, &[]),
        );
    }

    #[test]
    fn declared_provenance_ignores_uncertain_inputs() {
        // The executor *declares* Deterministic regardless of inputs.
        // Kernel invariant 2 separately propagates max-uncertainty at
        // emit time; that's not this trait's job.
        let exec = ScriptExecutor::new(["true"]);
        let inputs = [Provenance::Uncertain {
            source: SourceTag::Agent,
        }];
        assert_eq!(
            Executor::declared_provenance(&exec, &inputs),
            Provenance::Deterministic {
                source: SourceTag::Script,
            }
        );
    }

    #[tokio::test]
    async fn echo_hello_produces_artifact_with_expected_body() {
        // Verification bullet 1 (issue #354): a one-node Workflow with
        // ScriptExecutor { command: ["echo", "hello"] } runs and
        // produces an artifact with body "hello\n" and provenance
        // Deterministic { source: Script }.
        let exec = ScriptExecutor::new(["echo", "hello"]);
        let ctx = ctx();
        let node_id = ctx.node_id;
        let outputs = exec.execute(ctx).await.expect("script succeeds");

        assert_eq!(outputs.artifacts.len(), 1);
        let (_id, artifact) = &outputs.artifacts[0];

        // Provenance is the declared shape.
        assert_eq!(
            artifact.provenance,
            Provenance::Deterministic {
                source: SourceTag::Script,
            }
        );
        // Producer node is tagged.
        assert_eq!(artifact.produced_by_node, Some(node_id));
        // Single version, current_version aligned.
        assert_eq!(artifact.versions.len(), 1);
        assert_eq!(artifact.current_version, 1);

        // Body decodes back to stdout.
        let body =
            decode_inline_body(&artifact.versions[0].content_ref.uri).expect("inline body decodes");
        assert_eq!(body, b"hello\n");
    }

    #[tokio::test]
    async fn nonzero_exit_returns_script_failed() {
        // Verification bullet 2: exit code 1 → error propagated cleanly.
        // `sh -c "exit 1"` is portable across the unix testbeds CI
        // uses; on Windows this test would need a different incantation
        // but Onsager's CI is Linux-only today.
        let exec = ScriptExecutor::new(["sh", "-c", "echo oops 1>&2; exit 1"]);
        let err = exec.execute(ctx()).await.expect_err("nonzero exits");
        match err {
            ExecutorError::ScriptFailed { exit_code, stderr } => {
                assert_eq!(exit_code, 1);
                assert!(stderr.contains("oops"), "stderr captured: {stderr:?}");
            }
            other => panic!("expected ScriptFailed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn empty_command_is_a_failure() {
        let exec = ScriptExecutor {
            command: vec![],
            env: HashMap::new(),
            timeout_secs: 5,
        };
        let err = exec.execute(ctx()).await.expect_err("empty argv");
        assert!(matches!(err, ExecutorError::Failed(_)));
    }

    #[tokio::test]
    async fn spawn_failure_surfaces_as_failed() {
        // Pick a binary that doesn't exist — tokio's Command surfaces
        // the spawn failure as an io error, which the executor
        // translates into ExecutorError::Failed (not ScriptFailed —
        // the process never ran, there's no exit code).
        let exec = ScriptExecutor::new(["/nonexistent/onsager-test-binary"]);
        let err = exec.execute(ctx()).await.expect_err("spawn fails");
        match err {
            ExecutorError::Failed(msg) => assert!(msg.contains("spawn failed")),
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn timeout_kills_the_subprocess() {
        // 1-second timeout against a 10-second sleep — the executor
        // should return a Failed timeout error well before 10s.
        let exec = ScriptExecutor::new(["sh", "-c", "sleep 10"]).with_timeout_secs(1);
        let start = std::time::Instant::now();
        let err = exec.execute(ctx()).await.expect_err("timed out");
        let elapsed = start.elapsed();
        assert!(
            elapsed < Duration::from_secs(5),
            "timeout took too long: {elapsed:?}"
        );
        match err {
            ExecutorError::Failed(msg) => assert!(msg.contains("timed out")),
            other => panic!("expected Failed (timeout), got {other:?}"),
        }
    }

    #[tokio::test]
    async fn env_is_passed_to_subprocess() {
        // The subprocess starts with a cleared env; only `env` entries
        // should be visible. `sh -c 'echo $ONSAGER_TEST'` prints the
        // value, which the test reads back from the artifact body.
        let mut env = HashMap::new();
        env.insert("ONSAGER_TEST".into(), "marvin".into());
        // `sh` is on PATH on Linux; if `env_clear()` made PATH empty,
        // we'd need to set PATH here. Fortunately tokio's Command on
        // Linux still finds `sh` via execvp when the program name has
        // no slash (it consults the process's PATH, not the child's).
        let exec = ScriptExecutor::new(["sh", "-c", "echo $ONSAGER_TEST"]).with_env(env);
        let out = exec.execute(ctx()).await.expect("script succeeds");
        let body = decode_inline_body(&out.artifacts[0].1.versions[0].content_ref.uri).unwrap();
        assert_eq!(body, b"marvin\n");
    }

    #[test]
    fn decode_inline_body_rejects_non_inline_uris() {
        assert!(decode_inline_body("git://repo@abc").is_none());
        assert!(decode_inline_body("file:///tmp/x").is_none());
        assert!(decode_inline_body("").is_none());
    }

    #[test]
    fn decode_inline_body_rejects_invalid_base64() {
        assert!(decode_inline_body("inline:base64,@@@@").is_none());
    }

    #[test]
    fn inline_uri_roundtrips_through_decoder() {
        let payload: &[u8] = &[0, 1, 2, 250, 251, 252, b'\n'];
        let uri = format!("{INLINE_URI_PREFIX}{}", BASE64.encode(payload));
        assert_eq!(decode_inline_body(&uri).unwrap(), payload);
    }
}
