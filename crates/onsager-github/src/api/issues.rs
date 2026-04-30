//! Issue endpoints — `GET /repos/{owner}/{repo}/issues` paginated read,
//! plus the label toggle and issue-comment write paths used by the
//! portal's Phase 2 surfaces today.

use serde::{Deserialize, Serialize};

use crate::api::http::{client, GITHUB_API};
use crate::error::GithubError;

/// An issue as returned by `GET /repos/{owner}/{repo}/issues`. GitHub's
/// REST API includes pull requests in this endpoint; the
/// `pull_request` field distinguishes them.
#[derive(Debug, Clone, Deserialize)]
pub struct Issue {
    pub number: u64,
    pub title: String,
    pub state: String,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub labels: Vec<Label>,
    #[serde(default)]
    pub pull_request: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Label {
    pub name: String,
}

impl Issue {
    pub fn is_pull_request(&self) -> bool {
        self.pull_request.is_some()
    }

    pub fn has_label(&self, name: &str) -> bool {
        self.labels.iter().any(|l| l.name == name)
    }
}

/// Page through `GET /repos/{owner}/{repo}/issues` (which includes
/// PRs) until `cap` items are gathered or the listing ends. Pass
/// `token=None` for unauthenticated reads on public repos.
pub async fn list_recent_issues(
    token: Option<&str>,
    owner: &str,
    repo: &str,
    cap: usize,
) -> Result<Vec<Issue>, GithubError> {
    let mut out: Vec<Issue> = Vec::new();
    let mut page = 1u32;
    while out.len() < cap {
        let per_page = std::cmp::min(100, cap - out.len()).max(1);
        let url = format!(
            "{GITHUB_API}/repos/{owner}/{repo}/issues?state=all&per_page={per_page}&page={page}"
        );
        let mut req = client().get(&url);
        if let Some(tok) = token {
            req = req.bearer_auth(tok);
        }
        let resp = req.send().await?;
        if !resp.status().is_success() {
            return Err(GithubError::from_response(resp).await);
        }
        let batch: Vec<Issue> = resp
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

/// Toggle a label on an issue. `present=true` ensures the label is
/// set; `false` removes it.
pub async fn set_label(
    token: Option<&str>,
    owner: &str,
    repo: &str,
    issue_number: u64,
    label: &str,
    present: bool,
) -> Result<(), GithubError> {
    if present {
        let url = format!("{GITHUB_API}/repos/{owner}/{repo}/issues/{issue_number}/labels");
        let body = serde_json::json!({ "labels": [label] });
        let mut req = client().post(&url).json(&body);
        if let Some(tok) = token {
            req = req.bearer_auth(tok);
        }
        let resp = req.send().await?;
        if !resp.status().is_success() {
            return Err(GithubError::from_response(resp).await);
        }
    } else {
        let url = format!("{GITHUB_API}/repos/{owner}/{repo}/issues/{issue_number}/labels/{label}");
        let mut req = client().delete(&url);
        if let Some(tok) = token {
            req = req.bearer_auth(tok);
        }
        let resp = req.send().await?;
        // 404 means the label wasn't present — that's the desired end state.
        if !resp.status().is_success() && resp.status().as_u16() != 404 {
            return Err(GithubError::from_response(resp).await);
        }
    }
    Ok(())
}

/// Post a regular issue comment. Used for Deny-verdict rationale.
pub async fn post_issue_comment(
    token: Option<&str>,
    owner: &str,
    repo: &str,
    issue_number: u64,
    body: &str,
) -> Result<(), GithubError> {
    let url = format!("{GITHUB_API}/repos/{owner}/{repo}/issues/{issue_number}/comments");
    let payload = serde_json::json!({ "body": body });
    let mut req = client().post(&url).json(&payload);
    if let Some(tok) = token {
        req = req.bearer_auth(tok);
    }
    let resp = req.send().await?;
    if !resp.status().is_success() {
        return Err(GithubError::from_response(resp).await);
    }
    Ok(())
}
