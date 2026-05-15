//! [`ExecutorError`] — the error type runtime executors return.
//!
//! Executors fail in four shapes:
//!
//! - **`UnknownKind`** — the registry was asked to dispatch a kind it
//!   doesn't know. Surfaces from [`crate::dispatch`]; an individual
//!   `Executor::execute` impl will never construct this itself.
//! - **`Spine`** — a [`crate::SpineClient`] call failed. Most
//!   executors propagate this with `?`; the `#[from]` impl keeps the
//!   call sites quiet.
//! - **`ScriptFailed`** — the [`crate::script::ScriptExecutor`] ran a
//!   subprocess that exited non-zero. Typed because the kernel's
//!   downstream error-handling (and the operator UI) wants to surface
//!   exit code + captured stderr without re-parsing a free-text
//!   message. Other executors that wrap a subprocess shape (Verify in
//!   subprocess mode, Agent's sidecars) can reuse this variant rather
//!   than mint their own.
//! - **`Failed`** — anything else the executor wants to report. Free-
//!   text on purpose: forcing every executor's domain errors into a
//!   closed enum would re-introduce the catalog problem ADR 0012
//!   removes.

use thiserror::Error;

use crate::SpineError;

/// Error returned by a runtime executor's `execute()` method (or by
/// [`crate::dispatch`] when the kind isn't registered).
#[derive(Debug, Error)]
pub enum ExecutorError {
    /// The registry had no entry for the executor kind requested.
    #[error("no executor registered for kind `{0}`")]
    UnknownKind(String),

    /// A call into the spine port failed.
    #[error(transparent)]
    Spine(#[from] SpineError),

    /// A subprocess-shaped executor (Script today) exited non-zero.
    /// `stderr` is the captured stderr stream, lossily decoded to
    /// UTF-8; consumers that need the raw bytes should add a typed
    /// variant rather than parsing this back out.
    #[error("script exited with code {exit_code}: {stderr}")]
    ScriptFailed { exit_code: i32, stderr: String },

    /// Executor-specific failure — free-text reason.
    #[error("executor failed: {0}")]
    Failed(String),
}

impl ExecutorError {
    /// Build a `Failed` error from any displayable value.
    pub fn failed(msg: impl Into<String>) -> Self {
        Self::Failed(msg.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_kind_message_includes_kind() {
        let err = ExecutorError::UnknownKind("script".into());
        assert_eq!(err.to_string(), "no executor registered for kind `script`");
    }

    #[test]
    fn spine_error_converts_via_from() {
        let spine = SpineError::new("boom");
        let err: ExecutorError = spine.into();
        assert!(matches!(err, ExecutorError::Spine(_)));
        // Transparent forwarding — the spine error's display shows
        // through, no extra "executor:" prefix.
        assert_eq!(err.to_string(), "spine error: boom");
    }

    #[test]
    fn failed_helper_wraps_string() {
        let err = ExecutorError::failed("nope");
        assert_eq!(err.to_string(), "executor failed: nope");
    }

    #[test]
    fn script_failed_message_includes_exit_code_and_stderr() {
        let err = ExecutorError::ScriptFailed {
            exit_code: 7,
            stderr: "boom".into(),
        };
        assert_eq!(err.to_string(), "script exited with code 7: boom");
    }
}
