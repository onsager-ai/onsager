//! `GET /api/build-info` — runtime build descriptor for the dashboard.
//!
//! Used by the FTUE convergence (spec #398): the workspace-less `/chat`
//! entry renders an OSS banner when running on a developer's local
//! machine, suppresses it on Cloud. The signal is `is_oss` — `true` for
//! local OSS builds, `false` for the Cloud SaaS deploy. No auth: the
//! dashboard needs to read this before any user state is loaded.

use axum::Json;
use axum::extract::State;
use axum::response::{IntoResponse, Response};
use serde::Serialize;

use crate::state::AppState;

#[derive(Debug, Serialize)]
pub struct BuildInfo {
    /// `true` when this portal is running as an OSS self-host (no Cloud
    /// deployment marker). The dashboard renders the OSS banner only
    /// when this is `true`.
    pub is_oss: bool,
    /// Portal package version (Cargo.toml). Mostly diagnostic — surfaced
    /// in the sidebar footer in some build modes.
    pub version: &'static str,
}

/// GET /api/build-info — return the deployment descriptor.
pub async fn build_info(State(state): State<AppState>) -> Response {
    Json(compute_build_info(state.config.deployment.as_deref())).into_response()
}

/// The pure derivation, factored out for unit testing without an AppState.
fn compute_build_info(deployment: Option<&str>) -> BuildInfo {
    // Cloud deploys set `ONSAGER_DEPLOYMENT=cloud` in their env. Anything
    // else (unset, `oss`, a developer machine running `just dev`) reads
    // as OSS. Cheap, build-time-friendly, and a Cloud operator can flip a
    // single env var to reverse the default for a self-hosted enterprise
    // deploy that wants Cloud chrome.
    let is_oss = !deployment.is_some_and(|d| d.eq_ignore_ascii_case("cloud"));
    BuildInfo {
        is_oss,
        version: env!("CARGO_PKG_VERSION"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_to_oss_when_unset() {
        let bi = compute_build_info(None);
        assert!(bi.is_oss);
    }

    #[test]
    fn cloud_marker_flips_is_oss_off() {
        let bi = compute_build_info(Some("cloud"));
        assert!(!bi.is_oss);
        let bi = compute_build_info(Some("CLOUD"));
        assert!(!bi.is_oss, "case-insensitive match");
    }

    #[test]
    fn unknown_marker_reads_as_oss() {
        // A Cloud operator who wants OSS chrome can set the env var to
        // anything that isn't `cloud` — including the explicit `oss`.
        let bi = compute_build_info(Some("oss"));
        assert!(bi.is_oss);
        let bi = compute_build_info(Some("preview"));
        assert!(bi.is_oss);
    }

    #[test]
    fn version_matches_crate_version() {
        let bi = compute_build_info(None);
        assert_eq!(bi.version, env!("CARGO_PKG_VERSION"));
    }
}
