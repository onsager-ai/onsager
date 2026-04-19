//! Minimal GitHub REST helpers used by the portal.
//!
//! These are intentionally narrow — the portal only needs to: list a repo's
//! recent issues + PRs (backfill), post check runs (Phase 2), post review
//! comments (Phase 2 Deny), and toggle the `in-progress` label on the linked
//! spec issue (Phase 2 label-sync migration). Full Octocat-style coverage
//! lives elsewhere.

use serde::{Deserialize, Serialize};

const GITHUB_API: &str = "https://api.github.com";
const USER_AGENT: &str = "onsager-portal/0.1";

/// A pull request as returned by `GET /repos/{owner}/{repo}/pulls`.
#[derive(Debug, Clone, Deserialize)]
pub struct Pull {
    pub number: u64,
    pub title: String,
    pub state: String,
    #[serde(default)]
    pub merged_at: Option<String>,
    pub head: PullRef,
    pub base: PullRef,
    pub html_url: String,
    pub user: GithubUser,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PullRef {
    #[serde(rename = "ref")]
    pub ref_name: String,
    pub sha: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GithubUser {
    pub login: String,
}

/// An issue as returned by `GET /repos/{owner}/{repo}/issues`. GitHub's REST
/// API includes pull requests in this endpoint; the `pull_request` field
/// distinguishes them.
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

/// REST client. `token` is a PAT or installation-scoped token; passed via
/// `Authorization: Bearer ...`. When `token` is `None` only public unauth
/// endpoints work — fine for backfilling open-source repos in dev.
#[derive(Debug, Clone)]
pub struct Client {
    http: reqwest::Client,
    token: Option<String>,
}

impl Client {
    pub fn new(token: Option<String>) -> Self {
        Self {
            http: reqwest::Client::builder()
                .user_agent(USER_AGENT)
                .build()
                .expect("reqwest client"),
            token,
        }
    }

    fn auth(&self, mut req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        if let Some(tok) = &self.token {
            req = req.bearer_auth(tok);
        }
        req
    }

    /// Page through `GET /repos/{owner}/{repo}/issues` (which includes PRs)
    /// until `cap` items are gathered or the listing ends.
    pub async fn list_recent_issues(
        &self,
        owner: &str,
        repo: &str,
        cap: usize,
    ) -> anyhow::Result<Vec<Issue>> {
        let mut out: Vec<Issue> = Vec::new();
        let mut page = 1u32;
        while out.len() < cap {
            let per_page = std::cmp::min(100, cap - out.len()).max(1);
            let url = format!(
                "{GITHUB_API}/repos/{owner}/{repo}/issues?state=all&per_page={per_page}&page={page}"
            );
            let resp = self.auth(self.http.get(&url)).send().await?;
            if !resp.status().is_success() {
                anyhow::bail!("GitHub list issues failed ({}): {}", resp.status(), url);
            }
            let batch: Vec<Issue> = resp.json().await?;
            if batch.is_empty() {
                break;
            }
            out.extend(batch);
            page += 1;
        }
        out.truncate(cap);
        Ok(out)
    }

    /// Page through `GET /repos/{owner}/{repo}/pulls` similarly.
    pub async fn list_recent_pulls(
        &self,
        owner: &str,
        repo: &str,
        cap: usize,
    ) -> anyhow::Result<Vec<Pull>> {
        let mut out: Vec<Pull> = Vec::new();
        let mut page = 1u32;
        while out.len() < cap {
            let per_page = std::cmp::min(100, cap - out.len()).max(1);
            let url = format!(
                "{GITHUB_API}/repos/{owner}/{repo}/pulls?state=all&per_page={per_page}&page={page}"
            );
            let resp = self.auth(self.http.get(&url)).send().await?;
            if !resp.status().is_success() {
                anyhow::bail!("GitHub list pulls failed ({}): {}", resp.status(), url);
            }
            let batch: Vec<Pull> = resp.json().await?;
            if batch.is_empty() {
                break;
            }
            out.extend(batch);
            page += 1;
        }
        out.truncate(cap);
        Ok(out)
    }

    /// Toggle a label on an issue. `present=true` ensures the label is set;
    /// `false` removes it. Used by the Phase 2 label-sync migration.
    pub async fn set_label(
        &self,
        owner: &str,
        repo: &str,
        issue_number: u64,
        label: &str,
        present: bool,
    ) -> anyhow::Result<()> {
        if present {
            let url = format!("{GITHUB_API}/repos/{owner}/{repo}/issues/{issue_number}/labels");
            let body = serde_json::json!({ "labels": [label] });
            let resp = self.auth(self.http.post(&url).json(&body)).send().await?;
            if !resp.status().is_success() {
                anyhow::bail!("GitHub add label failed: {}", resp.status());
            }
        } else {
            let url =
                format!("{GITHUB_API}/repos/{owner}/{repo}/issues/{issue_number}/labels/{label}");
            let resp = self.auth(self.http.delete(&url)).send().await?;
            // 404 means the label wasn't present — that's the desired end state.
            if !resp.status().is_success() && resp.status().as_u16() != 404 {
                anyhow::bail!("GitHub remove label failed: {}", resp.status());
            }
        }
        Ok(())
    }

    /// Post a check run on a head SHA. Returns the created check-run id.
    pub async fn create_check_run(
        &self,
        owner: &str,
        repo: &str,
        head_sha: &str,
        name: &str,
        conclusion: CheckConclusion,
        summary: &str,
    ) -> anyhow::Result<u64> {
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
        let resp = self.auth(self.http.post(&url).json(&body)).send().await?;
        if !resp.status().is_success() {
            anyhow::bail!("GitHub create check run failed: {}", resp.status());
        }
        let v: serde_json::Value = resp.json().await?;
        Ok(v.get("id").and_then(|i| i.as_u64()).unwrap_or_default())
    }

    /// Post a regular issue comment. Used for Deny-verdict rationale.
    pub async fn post_issue_comment(
        &self,
        owner: &str,
        repo: &str,
        issue_number: u64,
        body: &str,
    ) -> anyhow::Result<()> {
        let url = format!("{GITHUB_API}/repos/{owner}/{repo}/issues/{issue_number}/comments");
        let payload = serde_json::json!({ "body": body });
        let resp = self
            .auth(self.http.post(&url).json(&payload))
            .send()
            .await?;
        if !resp.status().is_success() {
            anyhow::bail!("GitHub post comment failed: {}", resp.status());
        }
        Ok(())
    }
}

/// GitHub check-run conclusion variants the portal uses. Mirrors the
/// strings the API expects — kept narrow so we don't ship every variant
/// before there's a use site.
#[derive(Debug, Clone, Copy)]
pub enum CheckConclusion {
    Success,
    Failure,
    ActionRequired,
    Neutral,
}

impl CheckConclusion {
    fn as_str(self) -> &'static str {
        match self {
            CheckConclusion::Success => "success",
            CheckConclusion::Failure => "failure",
            CheckConclusion::ActionRequired => "action_required",
            CheckConclusion::Neutral => "neutral",
        }
    }
}
