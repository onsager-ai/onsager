//! Pull-request endpoints — paginated `GET /repos/{owner}/{repo}/pulls`
//! plus the `POST /repos/.../check-runs` write path used to record
//! gate verdicts.

use serde::Deserialize;

use crate::api::http::{client, GITHUB_API};
use crate::error::GithubError;

/// A pull request as returned by `GET /repos/{owner}/{repo}/pulls`.
#[derive(Debug, Clone, Deserialize)]
pub struct Pull {
    pub number: u64,
    pub title: String,
    pub state: String,
    #[serde(default)]
    pub merged_at: Option<String>,
    /// Present on merged PRs; this is the commit created *in the base
    /// branch* by the merge (as opposed to `head.sha`, which is the
    /// tip of the PR branch itself). `None` for unmerged PRs.
    #[serde(default)]
    pub merge_commit_sha: Option<String>,
    pub head: PullRef,
    pub base: PullRef,
    pub html_url: String,
    pub user: PullUser,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PullRef {
    #[serde(rename = "ref")]
    pub ref_name: String,
    pub sha: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PullUser {
    pub login: String,
}

/// Page through `GET /repos/{owner}/{repo}/pulls` until `cap` items
/// are gathered or the listing ends.
pub async fn list_recent_pulls(
    token: Option<&str>,
    owner: &str,
    repo: &str,
    cap: usize,
) -> Result<Vec<Pull>, GithubError> {
    let mut out: Vec<Pull> = Vec::new();
    let mut page = 1u32;
    while out.len() < cap {
        let per_page = std::cmp::min(100, cap - out.len()).max(1);
        let url = format!(
            "{GITHUB_API}/repos/{owner}/{repo}/pulls?state=all&per_page={per_page}&page={page}"
        );
        let mut req = client().get(&url);
        if let Some(tok) = token {
            req = req.bearer_auth(tok);
        }
        let resp = req.send().await?;
        if !resp.status().is_success() {
            return Err(GithubError::from_response(resp).await);
        }
        let batch: Vec<Pull> = resp
            .json()
            .await
            .map_err(|e| GithubError::Decode(e.to_string()))?;
        if batch.is_empty() {
            break;
        }
        out.extend(batch);
        page += 1;
    }
    out.truncate(cap);
    Ok(out)
}

/// GitHub check-run conclusion variants. Mirrors the strings the API
/// expects — kept narrow so we don't ship variants without a use site.
#[derive(Debug, Clone, Copy)]
pub enum CheckConclusion {
    Success,
    Failure,
    ActionRequired,
    Neutral,
}

impl CheckConclusion {
    pub fn as_str(self) -> &'static str {
        match self {
            CheckConclusion::Success => "success",
            CheckConclusion::Failure => "failure",
            CheckConclusion::ActionRequired => "action_required",
            CheckConclusion::Neutral => "neutral",
        }
    }
}

/// Post a check run on a head SHA. Returns the created check-run id.
pub async fn create_check_run(
    token: Option<&str>,
    owner: &str,
    repo: &str,
    head_sha: &str,
    name: &str,
    conclusion: CheckConclusion,
    summary: &str,
) -> Result<u64, GithubError> {
    let url = format!("{GITHUB_API}/repos/{owner}/{repo}/check-runs");
    let body = serde_json::json!({
        "name": name,
        "head_sha": head_sha,
        "status": "completed",
        "conclusion": conclusion.as_str(),
        "output": {
            "title": name,
            "summary": summary,
        }
    });
    let mut req = client().post(&url).json(&body);
    if let Some(tok) = token {
        req = req.bearer_auth(tok);
    }
    let resp = req.send().await?;
    if !resp.status().is_success() {
        return Err(GithubError::from_response(resp).await);
    }
    let v: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| GithubError::Decode(e.to_string()))?;
    Ok(v.get("id").and_then(|i| i.as_u64()).unwrap_or_default())
}
