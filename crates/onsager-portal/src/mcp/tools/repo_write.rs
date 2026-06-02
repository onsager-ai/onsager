//! MCP tool `request_repo_write` — gate + mint an on-demand, per-repo
//! write token (spec #548, Part of #544).
//!
//! In a workspace-scoped run the agent is handed *read* access to the
//! workspace's bound repo set (`ONSAGER_REPOS`, #546). Binding a repo
//! grants **candidacy, not write consent**. When the agent decides it
//! needs to push + open a PR to a bound-but-unpinned repo, it calls this
//! tool. Portal:
//!
//! 1. Verifies the repo is actually bound to the workspace (candidacy).
//! 2. Emits `forge.gate_requested` with `GatePoint::RepoWrite`, keyed on
//!    the gate id, carrying the target repo in `GateContext.extra`.
//! 3. Awaits the matching `synodic.gate_verdict` on the same stream.
//! 4. On `Allow`, mints a `contents:write` + `pull_requests:write` token
//!    scoped to that one repo and returns an authenticated remote. On
//!    `Deny` / `Escalate` / verdict-timeout, **no token is minted** and
//!    the write is parked per `SYNODIC_FAIL_POLICY`.
//!
//! The pinned repo (named in the trigger) is pre-approved — the pin *is*
//! the consent — and uses the unchanged push/PR path, so it never reaches
//! this tool (the #548 regression bar: no added gate hop for pinned
//! single-repo runs).
//!
//! Seam note: this is a spine round-trip (emit an event, read an event),
//! not a subsystem→subsystem HTTP call. Portal owns the GitHub
//! credentials, so the mint stays here; Synodic owns the verdict.

use std::time::{Duration, Instant};

use onsager_artifact::{ArtifactId, ArtifactState, Kind};
use onsager_spine::EventMetadata;
use onsager_spine::EventStore;
use onsager_spine::factory_event::{FactoryEventKind, GatePoint};
use onsager_spine::protocol::{GateContext, GateRequest, GateVerdict, ProposedAction};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

use crate::auth::AuthUser;
use crate::repo_access::{self, WriteTokenError};
use crate::state::AppState;

use super::super::{ToolError, ToolResult};
use super::require_workspace_access;

/// How long to wait for Synodic's verdict before applying the fail policy.
/// Overridable via `PORTAL_REPO_WRITE_GATE_TIMEOUT_SECS`.
const DEFAULT_GATE_TIMEOUT_SECS: u64 = 30;
/// Poll cadence while awaiting the verdict on the gate stream.
const POLL_INTERVAL: Duration = Duration::from_millis(250);

#[derive(Debug, Deserialize, JsonSchema)]
pub struct RequestRepoWriteArgs {
    /// Workspace that owns the run. The caller's MCP token scope is
    /// enforced against this value before anything is written.
    pub workspace_id: String,
    /// The session/run requesting write access — keys the gate to the run
    /// so the verdict and any audit trail attribute to it.
    pub session_id: String,
    /// Target repo owner (org or user login).
    pub owner: String,
    /// Target repo short name (no owner prefix).
    pub name: String,
    /// Artifact the run is shaping, when known — scopes the gate event so
    /// the dashboard timeline attaches it to the run. Optional; a
    /// session-derived synthetic id is used when absent.
    #[serde(default)]
    pub artifact_id: Option<String>,
}

/// Fail-open/closed posture applied when no verdict arrives in time.
/// Mirrors forge's `SYNODIC_FAIL_POLICY` semantics (default `escalate`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FailPolicy {
    Escalate,
    Deny,
    Allow,
}

/// Parse `SYNODIC_FAIL_POLICY`. Unknown / unset → `Escalate` (the safe
/// default: park the decision non-blockingly rather than fail open).
fn parse_fail_policy(raw: Option<&str>) -> FailPolicy {
    match raw.map(|s| s.trim().to_ascii_lowercase()).as_deref() {
        Some("deny") => FailPolicy::Deny,
        Some("allow") => FailPolicy::Allow,
        _ => FailPolicy::Escalate,
    }
}

/// What the tool does next, derived from the verdict (or the timeout
/// fallback). Kept pure so the allow/deny/park mapping is unit-testable
/// — and the #548 mutation check (force `deny`, the push must not occur)
/// lives on `decide_from_verdict`.
#[derive(Debug, PartialEq, Eq)]
enum WriteDecision {
    /// Mint the write token and hand back an authenticated remote.
    Grant,
    /// Refuse — no token minted.
    Deny(String),
    /// Park non-blockingly — no token minted; the operator decides.
    Park(String),
}

fn decide_from_verdict(verdict: &GateVerdict) -> WriteDecision {
    match verdict {
        GateVerdict::Allow => WriteDecision::Grant,
        GateVerdict::Deny { reason } => WriteDecision::Deny(reason.clone()),
        GateVerdict::Escalate { context } => WriteDecision::Park(context.reason.clone()),
        // A repo-write request is allow-or-deny; there is no action to
        // rewrite, so a `modify` verdict is treated conservatively as a
        // park rather than silently granting.
        GateVerdict::Modify { .. } => {
            WriteDecision::Park("verdict was 'modify'; repo-write has no action to rewrite".into())
        }
    }
}

fn decide_on_timeout(policy: FailPolicy) -> WriteDecision {
    match policy {
        FailPolicy::Escalate => WriteDecision::Park(
            "no synodic verdict within deadline; escalated per SYNODIC_FAIL_POLICY".into(),
        ),
        FailPolicy::Deny => WriteDecision::Deny(
            "no synodic verdict within deadline; denied per SYNODIC_FAIL_POLICY".into(),
        ),
        FailPolicy::Allow => WriteDecision::Grant,
    }
}

/// Authenticated HTTPS remote the agent uses to `git push`. The token
/// rides in the `x-access-token` userinfo per GitHub's installation-token
/// convention, percent-encoded so a reserved character can't truncate or
/// corrupt the URL — same defensive shape the read side uses for clone
/// URLs (`stiglab/src/server/repo_env.rs`). GitHub installation tokens are
/// URL-safe today; encoding keeps this correct if that ever changes.
fn authenticated_remote(owner: &str, name: &str, token: &str) -> String {
    format!(
        "https://x-access-token:{}@github.com/{owner}/{name}.git",
        onsager_agent_spawn::urlencode(token)
    )
}

/// Poll the gate stream for the matching `synodic.gate_verdict`. Returns
/// `None` on timeout (the caller then applies the fail policy). The
/// request and verdict share `gate_id` as their stream id, so this read
/// is bounded to that one stream.
async fn await_verdict(
    store: &EventStore,
    gate_id: &str,
    timeout: Duration,
) -> Option<GateVerdict> {
    let start = Instant::now();
    loop {
        match store.query_ext_stream(gate_id).await {
            Ok(rows) => {
                for row in &rows {
                    if row.event_type != "synodic.gate_verdict" {
                        continue;
                    }
                    if let Ok(FactoryEventKind::SynodicGateVerdict {
                        gate_id: verdict_gate_id,
                        verdict,
                        ..
                    }) = serde_json::from_value::<FactoryEventKind>(row.data.clone())
                        && verdict_gate_id == gate_id
                    {
                        return Some(verdict);
                    }
                }
            }
            // A transient read error must not look like "no verdict yet":
            // log it so a persistent failure is visible (it would
            // otherwise present as a timeout, and under SYNODIC_FAIL_POLICY
            // = allow that is a silent fail-open). Keep polling — a blip
            // recovers, and the deadline still bounds the wait.
            Err(e) => {
                tracing::warn!(
                    %gate_id,
                    "request_repo_write: gate-stream read failed while awaiting verdict: {e}"
                );
            }
        }
        if start.elapsed() >= timeout {
            return None;
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

fn gate_timeout() -> Duration {
    let secs = std::env::var("PORTAL_REPO_WRITE_GATE_TIMEOUT_SECS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(DEFAULT_GATE_TIMEOUT_SECS);
    Duration::from_secs(secs)
}

pub async fn request_repo_write(state: &AppState, auth_user: &AuthUser, args: Value) -> ToolResult {
    let args: RequestRepoWriteArgs = serde_json::from_value(args)
        .map_err(|e| ToolError::InvalidParams(format!("invalid request_repo_write args: {e}")))?;
    require_workspace_access(&state.pool, auth_user, &args.workspace_id).await?;

    let target_repo = format!("{}/{}", args.owner, args.name);

    // Candidacy check: an unbound repo can never be written to, gate or
    // no gate. Resolve first so we don't emit a gate event for a repo
    // we'd reject anyway.
    if repo_access::resolve_install_for_repo(state, &args.workspace_id, &args.owner, &args.name)
        .await
        .is_none()
    {
        return Ok(serde_json::json!({
            "granted": false,
            "status": "repo_not_bound",
            "reason": format!(
                "{target_repo} is not bound to this workspace; binding grants candidacy for writes"
            ),
        }));
    }

    // Emit the RepoWrite gate request. The artifact id scopes the event
    // to the run when known; otherwise a session-derived synthetic id.
    let gate_id = Uuid::new_v4().to_string();
    let artifact_id = ArtifactId::new(
        args.artifact_id
            .clone()
            .unwrap_or_else(|| format!("session:{}", args.session_id)),
    );
    let extra = serde_json::json!({
        "target_repo": target_repo,
        "session_id": args.session_id,
    });
    let request = GateRequest {
        context: GateContext {
            gate_point: GatePoint::RepoWrite,
            artifact_id: artifact_id.clone(),
            artifact_kind: Kind::PullRequest,
            current_state: ArtifactState::InProgress,
            target_state: None,
            extra: Some(extra.clone()),
        },
        proposed_action: ProposedAction {
            description: format!("Push a branch and open a pull request to {target_repo}"),
            payload: extra,
        },
    };
    let event = FactoryEventKind::ForgeGateRequested {
        gate_id: gate_id.clone(),
        artifact_id,
        gate_point: GatePoint::RepoWrite,
        request: Some(request),
    };
    // Attribute the gate request to the authenticated principal making
    // the call, not a fixed subsystem string — mirrors `cancel_run` and
    // keeps the audit trail pointed at who asked for write access.
    let metadata = EventMetadata {
        correlation_id: None,
        causation_id: None,
        actor: auth_user.user_id.clone(),
    };
    state
        .spine
        .append_ext(
            &args.workspace_id,
            &gate_id,
            "forge",
            event.event_type(),
            serde_json::to_value(&event).map_err(|e| {
                ToolError::Internal(format!("failed to serialize gate request: {e}"))
            })?,
            &metadata,
            None,
        )
        .await
        .map_err(|e| {
            tracing::error!("request_repo_write: emit forge.gate_requested failed: {e}");
            ToolError::Internal(format!("failed to emit gate request: {e}"))
        })?;

    // Await the verdict; fall back to the configured fail policy on timeout.
    let decision = match await_verdict(&state.spine, &gate_id, gate_timeout()).await {
        Some(verdict) => decide_from_verdict(&verdict),
        None => decide_on_timeout(parse_fail_policy(
            std::env::var("SYNODIC_FAIL_POLICY").ok().as_deref(),
        )),
    };

    match decision {
        WriteDecision::Grant => {
            match repo_access::mint_repo_write_token(
                state,
                &args.workspace_id,
                &args.owner,
                &args.name,
            )
            .await
            {
                // The token is returned only once, embedded in `remote`
                // (sufficient for `git push`). Returning it as a separate
                // field too would duplicate a secret and widen the chance
                // a client logs or persists it.
                Ok(token) => Ok(serde_json::json!({
                    "granted": true,
                    "status": "granted",
                    "gate_id": gate_id,
                    "remote": authenticated_remote(&args.owner, &args.name, &token.token),
                    "expires_at": token.expires_at,
                })),
                Err(WriteTokenError::RepoNotBound) => Ok(serde_json::json!({
                    "granted": false,
                    "status": "repo_not_bound",
                    "gate_id": gate_id,
                    "reason": format!("{target_repo} is not bound to this workspace"),
                })),
                Err(e) => {
                    tracing::error!("request_repo_write: gate allowed but mint failed: {e}");
                    Err(ToolError::Internal(format!(
                        "gate allowed but write-token mint failed: {e}"
                    )))
                }
            }
        }
        WriteDecision::Deny(reason) => Ok(serde_json::json!({
            "granted": false,
            "status": "denied",
            "gate_id": gate_id,
            "reason": reason,
        })),
        WriteDecision::Park(reason) => Ok(serde_json::json!({
            "granted": false,
            "status": "parked",
            "gate_id": gate_id,
            "reason": reason,
        })),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use onsager_spine::protocol::EscalationContext;

    #[test]
    fn fail_policy_parse_defaults_to_escalate() {
        assert_eq!(parse_fail_policy(None), FailPolicy::Escalate);
        assert_eq!(parse_fail_policy(Some("nonsense")), FailPolicy::Escalate);
        assert_eq!(parse_fail_policy(Some("escalate")), FailPolicy::Escalate);
        assert_eq!(parse_fail_policy(Some(" DENY ")), FailPolicy::Deny);
        assert_eq!(parse_fail_policy(Some("allow")), FailPolicy::Allow);
    }

    #[test]
    fn deny_verdict_never_grants() {
        // #548 mutation check: a `deny` verdict must not mint a token.
        // Flip this arm to `WriteDecision::Grant` and the assert fails —
        // the push would proceed under a denial.
        let decision = decide_from_verdict(&GateVerdict::Deny {
            reason: "org not on the write allowlist".into(),
        });
        assert_eq!(
            decision,
            WriteDecision::Deny("org not on the write allowlist".into())
        );
        assert_ne!(decision, WriteDecision::Grant);
    }

    #[test]
    fn allow_verdict_grants() {
        assert_eq!(
            decide_from_verdict(&GateVerdict::Allow),
            WriteDecision::Grant
        );
    }

    #[test]
    fn escalate_and_modify_verdicts_park() {
        let escalate = decide_from_verdict(&GateVerdict::Escalate {
            context: EscalationContext {
                escalation_id: "esc_1".into(),
                reason: "needs human review".into(),
                target: "user".into(),
                timeout_at: chrono::Utc::now(),
            },
        });
        assert_eq!(escalate, WriteDecision::Park("needs human review".into()));

        let modify = decide_from_verdict(&GateVerdict::Modify {
            new_action: ProposedAction {
                description: "x".into(),
                payload: serde_json::json!({}),
            },
        });
        assert!(matches!(modify, WriteDecision::Park(_)));
    }

    #[test]
    fn timeout_fallback_follows_fail_policy() {
        assert!(matches!(
            decide_on_timeout(FailPolicy::Escalate),
            WriteDecision::Park(_)
        ));
        assert!(matches!(
            decide_on_timeout(FailPolicy::Deny),
            WriteDecision::Deny(_)
        ));
        assert_eq!(decide_on_timeout(FailPolicy::Allow), WriteDecision::Grant);
    }

    #[test]
    fn authenticated_remote_embeds_token() {
        let remote = authenticated_remote("acme", "widgets", "ghs_tok");
        assert_eq!(
            remote,
            "https://x-access-token:ghs_tok@github.com/acme/widgets.git"
        );
    }
}
