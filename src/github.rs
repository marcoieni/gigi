use anyhow::Context as _;
use camino::{Utf8Path, Utf8PathBuf};
use serde_json::Value;

use crate::{checkout::parse_github_pr_url, cmd::Cmd};

#[derive(Debug, Clone)]
pub struct NotificationThread {
    pub thread_id: String,
    pub unread: bool,
    pub reason: Option<String>,
    pub updated_at: String,
    pub repository: String,
    pub subject_type: Option<String>,
    pub subject_title: String,
    pub subject_url: Option<String>,
    pub pr_url: Option<String>,
}

#[derive(Debug, Clone)]
pub struct AuthoredPrSummary {
    pub pr_url: String,
    pub repository: String,
    pub title: String,
    pub updated_at: String,
}

#[derive(Debug, Clone)]
pub struct PrDetails {
    pub pr_url: String,
    pub owner: String,
    pub repo: String,
    pub number: i64,
    pub state: String,
    pub title: String,
    pub head_ref: String,
    pub base_ref: String,
    pub head_sha: String,
    pub created_at: String,
    pub updated_at: String,
}

pub fn fetch_notifications() -> anyhow::Result<Vec<NotificationThread>> {
    let output = Cmd::new("gh", ["api", "/notifications", "--paginate", "--slurp"]).run()?;
    output.ensure_success("❌ Failed to fetch notifications")?;

    if output.stdout().trim().is_empty() {
        return Ok(Vec::new());
    }

    let value: Value =
        serde_json::from_str(output.stdout()).context("Invalid notifications JSON")?;

    let pages: Vec<Value> = match value {
        Value::Array(items) if items.iter().all(Value::is_array) => items,
        Value::Array(items) => vec![Value::Array(items)],
        _ => vec![],
    };

    let mut results = Vec::new();
    for page in pages {
        let Value::Array(entries) = page else {
            continue;
        };

        for entry in entries {
            let thread_id = entry
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            if thread_id.is_empty() {
                continue;
            }

            let unread = entry
                .get("unread")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let reason = entry
                .get("reason")
                .and_then(Value::as_str)
                .map(ToString::to_string);
            let updated_at = entry
                .get("updated_at")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();

            let repository = entry
                .get("repository")
                .and_then(|v| v.get("full_name"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            if repository.is_empty() {
                continue;
            }

            let subject_type = entry
                .get("subject")
                .and_then(|v| v.get("type"))
                .and_then(Value::as_str)
                .map(ToString::to_string);
            let subject_title = entry
                .get("subject")
                .and_then(|v| v.get("title"))
                .and_then(Value::as_str)
                .unwrap_or("(untitled)")
                .to_string();
            let raw_subject_url = entry
                .get("subject")
                .and_then(|v| v.get("url"))
                .and_then(Value::as_str)
                .map(ToString::to_string);

            let pr_url = raw_subject_url.as_deref().and_then(api_url_to_pr_url);
            let subject_url = raw_subject_url
                .as_deref()
                .and_then(api_url_to_html_url)
                .or(raw_subject_url);

            results.push(NotificationThread {
                thread_id,
                unread,
                reason,
                updated_at,
                repository,
                subject_type,
                subject_title,
                subject_url,
                pr_url,
            });
        }
    }

    Ok(results)
}

pub fn fetch_authored_open_prs() -> anyhow::Result<Vec<AuthoredPrSummary>> {
    let output = Cmd::new(
        "gh",
        [
            "search",
            "prs",
            "--author",
            "@me",
            "--state",
            "open",
            "--limit",
            "200",
            "--json",
            "url,title,updatedAt,repository",
        ],
    )
    .run()?;

    output.ensure_success("❌ Failed to fetch authored pull requests")?;
    if output.stdout().trim().is_empty() {
        return Ok(Vec::new());
    }

    let value: Value = serde_json::from_str(output.stdout()).context("Invalid authored PR JSON")?;
    let mut results = Vec::new();

    let Value::Array(items) = value else {
        return Ok(results);
    };

    for item in items {
        let pr_url = item
            .get("url")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        if pr_url.is_empty() {
            continue;
        }

        let title = item
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("(untitled)")
            .to_string();
        let updated_at = item
            .get("updatedAt")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let repository = item
            .get("repository")
            .and_then(|v| v.get("nameWithOwner"))
            .and_then(Value::as_str)
            .map(ToString::to_string)
            .or_else(|| {
                item.get("repository")
                    .and_then(|v| v.get("fullName"))
                    .and_then(Value::as_str)
                    .map(ToString::to_string)
            })
            .unwrap_or_else(|| parse_repo_from_pr_url(&pr_url).unwrap_or_default());

        if repository.is_empty() {
            continue;
        }

        results.push(AuthoredPrSummary {
            pr_url,
            repository,
            title,
            updated_at,
        });
    }

    Ok(results)
}

pub fn fetch_pr_details(pr_url: &str) -> anyhow::Result<PrDetails> {
    let output = Cmd::new(
        "gh",
        [
            "pr",
            "view",
            pr_url,
            "--json",
            "title,url,state,headRefName,headRefOid,baseRefName,createdAt,updatedAt,number",
        ],
    )
    .run()?;

    output.ensure_success(format!("❌ Failed to fetch PR details for {pr_url}"))?;
    anyhow::ensure!(
        !output.stdout().trim().is_empty(),
        "❌ Failed to fetch PR details for {pr_url}: empty output"
    );

    let value: Value = serde_json::from_str(output.stdout())?;
    let canonical_pr_url = value
        .get("url")
        .and_then(Value::as_str)
        .unwrap_or(pr_url)
        .to_string();

    let parsed = parse_github_pr_url(&canonical_pr_url)?;
    let number = i64::try_from(parsed.number)
        .with_context(|| format!("PR number is too large for i64: {}", parsed.number))?;
    let state = value
        .get("state")
        .and_then(Value::as_str)
        .unwrap_or("OPEN")
        .to_string();

    let title = value
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or("(untitled)")
        .to_string();
    let head_ref = value
        .get("headRefName")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let base_ref = value
        .get("baseRefName")
        .and_then(Value::as_str)
        .unwrap_or("main")
        .to_string();
    let head_sha = value
        .get("headRefOid")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let created_at = value
        .get("createdAt")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let updated_at = value
        .get("updatedAt")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();

    Ok(PrDetails {
        pr_url: canonical_pr_url,
        owner: parsed.owner,
        repo: parsed.repo,
        number,
        state,
        title,
        head_ref,
        base_ref,
        head_sha,
        created_at,
        updated_at,
    })
}

pub fn mark_notification_done(thread_id: &str) -> anyhow::Result<()> {
    let endpoint = format!("/notifications/threads/{thread_id}");
    let output = Cmd::new("gh", ["api", "-X", "DELETE", &endpoint]).run()?;
    output.ensure_success("❌ Failed to mark notification thread as done")?;
    Ok(())
}

pub fn ensure_local_repo(owner: &str, repo: &str) -> anyhow::Result<Utf8PathBuf> {
    let repo_dir = local_repo_dir(owner, repo)?;
    if repo_dir.exists() {
        anyhow::ensure!(
            repo_dir.join(".git").exists(),
            "❌ Path exists but is not a git repository: {repo_dir}"
        );
        return Ok(repo_dir);
    }

    let parent = repo_dir
        .parent()
        .context("Failed to compute repository parent directory")?;
    std::fs::create_dir_all(parent).with_context(|| format!("Failed to create {parent}"))?;

    let repo_name = format!("{owner}/{repo}");
    let output = Cmd::new("gh", ["repo", "clone", &repo_name, repo_dir.as_str()]).run()?;
    output.ensure_success(format!("❌ Failed to clone repository {repo_name}"))?;

    Ok(repo_dir)
}

pub fn local_repo_dir(owner: &str, repo: &str) -> anyhow::Result<Utf8PathBuf> {
    let home = std::env::var("HOME").context("HOME env var is not set")?;
    Ok(Utf8PathBuf::from(home).join("proj").join(owner).join(repo))
}

pub fn checkout_pr(repo_dir: &Utf8Path, pr_url: &str) -> anyhow::Result<()> {
    let output = Cmd::new("gh", ["pr", "checkout", pr_url])
        .with_current_dir(repo_dir)
        .run()?;
    output.ensure_success("❌ Failed to checkout PR")?;
    Ok(())
}

pub fn current_branch(repo_dir: &Utf8Path) -> anyhow::Result<String> {
    let output = Cmd::new("git", ["branch", "--show-current"])
        .with_current_dir(repo_dir)
        .run()?;
    output.ensure_success("❌ Failed to get current branch")?;
    Ok(output.stdout().to_string())
}

pub fn is_clean_repo(repo_dir: &Utf8Path) -> anyhow::Result<bool> {
    let output = Cmd::new("git", ["status", "--porcelain"])
        .with_current_dir(repo_dir)
        .run()?;
    output.ensure_success("❌ Failed to check repository status")?;
    Ok(output.stdout().trim().is_empty())
}

pub fn default_branch(repo_dir: &Utf8Path) -> anyhow::Result<String> {
    let output = Cmd::new(
        "gh",
        [
            "repo",
            "view",
            "--json",
            "defaultBranchRef",
            "-q",
            ".defaultBranchRef.name",
        ],
    )
    .with_current_dir(repo_dir)
    .run()?;

    output.ensure_success("❌ Failed to detect default branch")?;
    anyhow::ensure!(
        !output.stdout().trim().is_empty(),
        "❌ Failed to detect default branch: empty output"
    );

    Ok(output.stdout().to_string())
}

pub fn checkout_branch(repo_dir: &Utf8Path, branch: &str) -> anyhow::Result<()> {
    let output = Cmd::new("git", ["checkout", branch])
        .with_current_dir(repo_dir)
        .run()?;
    output.ensure_success(format!("❌ Failed to checkout branch '{branch}'"))?;
    Ok(())
}

pub fn pull_ff_only(repo_dir: &Utf8Path) -> anyhow::Result<()> {
    let output = Cmd::new("git", ["pull", "--ff-only"])
        .with_current_dir(repo_dir)
        .run()?;
    output.ensure_success("❌ Failed to pull default branch")?;
    Ok(())
}

fn api_url_to_pr_url(api_url: &str) -> Option<String> {
    let path = api_url
        .trim()
        .strip_prefix("https://api.github.com/")
        .unwrap_or(api_url)
        .strip_prefix("repos/")?;

    let parts: Vec<&str> = path.split('/').collect();
    if parts.len() < 4 || parts[2] != "pulls" {
        return None;
    }

    Some(format!(
        "https://github.com/{}/{}/pull/{}",
        parts[0], parts[1], parts[3]
    ))
}

fn api_url_to_html_url(api_url: &str) -> Option<String> {
    let path = api_url
        .trim()
        .strip_prefix("https://api.github.com/")
        .unwrap_or(api_url);

    if let Some(path) = path.strip_prefix("repos/") {
        let parts: Vec<&str> = path.split('/').collect();
        if parts.len() < 4 {
            return None;
        }

        let owner = parts[0];
        let repo = parts[1];
        let kind = parts[2];
        let number = parts[3];

        let route = match kind {
            "pulls" => format!("pull/{number}"),
            "issues" => format!("issues/{number}"),
            "discussions" => format!("discussions/{number}"),
            "commits" => format!("commit/{number}"),
            _ => return None,
        };

        return Some(format!("https://github.com/{owner}/{repo}/{route}"));
    }

    Some(api_url.to_string())
}

fn parse_repo_from_pr_url(pr_url: &str) -> Option<String> {
    let parsed = parse_github_pr_url(pr_url).ok()?;
    Some(format!("{}/{}", parsed.owner, parsed.repo))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_api_pr_url() {
        let pr = api_url_to_pr_url("https://api.github.com/repos/o/r/pulls/123").unwrap();
        assert_eq!(pr, "https://github.com/o/r/pull/123");
    }

    #[test]
    fn converts_api_issue_url() {
        let issue = api_url_to_html_url("https://api.github.com/repos/o/r/issues/123").unwrap();
        assert_eq!(issue, "https://github.com/o/r/issues/123");
    }

    #[test]
    fn leaves_html_url_unchanged() {
        let issue = api_url_to_html_url("https://github.com/o/r/issues/123").unwrap();
        assert_eq!(issue, "https://github.com/o/r/issues/123");
    }

    #[test]
    fn parse_repo_from_pr_url_works() {
        let repo = parse_repo_from_pr_url("https://github.com/o/r/pull/5").unwrap();
        assert_eq!(repo, "o/r");
    }
}
