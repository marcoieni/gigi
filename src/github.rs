use std::collections::HashMap;
use std::fmt::Write as _;

use anyhow::Context as _;
use camino::{Utf8Path, Utf8PathBuf};
use serde_json::Value;
use tokio::fs;

use crate::{
    checkout::parse_github_pr_url,
    cmd::{Cmd, CmdOutput},
};

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
    pub issue_api_url: Option<String>,
    pub issue_state: Option<String>,
}

#[derive(Debug, Clone)]
pub struct LocalPrRepo {
    pub repo_dir: Utf8PathBuf,
    pub details: PrDetails,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GitHubRepoRef {
    owner: String,
    repo: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CloneTarget {
    origin: GitHubRepoRef,
    upstream: Option<GitHubRepoRef>,
}

#[derive(Debug, Clone)]
pub struct AuthoredPrSummary {
    pub pr_url: String,
    pub repository: String,
    pub title: String,
    pub updated_at: String,
    pub is_open: bool,
    pub is_draft: bool,
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
    pub is_archived: bool,
    pub author_login: Option<String>,
    pub head_repo_owner: Option<String>,
    pub head_repo_name: Option<String>,
    pub is_cross_repository: bool,
    pub is_draft: bool,
}

pub async fn fetch_notifications(since: Option<&str>) -> anyhow::Result<Vec<NotificationThread>> {
    let endpoint = match since {
        Some(since) => format!("/notifications?since={since}"),
        None => "/notifications".to_string(),
    };
    let output = Cmd::new("gh", ["api", &endpoint, "--paginate", "--slurp"])
        .run()
        .await?;
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

            let pr_url = raw_subject_url
                .as_deref()
                .and_then(|url| api_url_to_pr_url(url, subject_type.as_deref()));
            let issue_api_url = match subject_type.as_deref() {
                Some("Issue") => raw_subject_url.clone(),
                _ => None,
            };
            let subject_url = raw_subject_url
                .as_deref()
                .and_then(api_url_to_html_url)
                .or(raw_subject_url.clone());

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
                issue_api_url,
                issue_state: None,
            });
        }
    }

    Ok(results)
}

pub async fn fetch_authored_prs(since: Option<&str>) -> anyhow::Result<Vec<AuthoredPrSummary>> {
    let mut args = vec![
        "search",
        "prs",
        "--author",
        "@me",
        "--limit",
        "200",
        "--json",
        "url,title,updatedAt,repository,state,isDraft",
    ];
    let updated_filter;
    if let Some(since) = since {
        updated_filter = format!(">={since}");
        args.push("--updated");
        args.push(&updated_filter);
    }
    let output = Cmd::new("gh", args).run().await?;

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
        let is_open = item
            .get("state")
            .and_then(Value::as_str)
            .map(|s| s.eq_ignore_ascii_case("open"))
            .unwrap_or(true);
        let is_draft = item
            .get("isDraft")
            .and_then(Value::as_bool)
            .unwrap_or(false);
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
            is_open,
            is_draft,
        });
    }

    Ok(results)
}

pub async fn fetch_pr_details(pr_url: &str) -> anyhow::Result<PrDetails> {
    let output = Cmd::new(
        "gh",
        [
            "pr",
            "view",
            pr_url,
            "--json",
            "title,url,state,isDraft,headRefName,headRefOid,baseRefName,createdAt,updatedAt,number,author,headRepository,headRepositoryOwner,isCrossRepository",
        ],
    )
    .run()
    .await?;

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
    let is_archived = fetch_repository_archived(&parsed.owner, &parsed.repo).await?;
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
    let author_login = value
        .get("author")
        .and_then(|v| v.get("login"))
        .and_then(Value::as_str)
        .map(ToString::to_string);
    let head_repo_owner = value
        .get("headRepositoryOwner")
        .and_then(|v| v.get("login"))
        .and_then(Value::as_str)
        .map(ToString::to_string);
    let head_repo_name = value
        .get("headRepository")
        .and_then(|v| v.get("name"))
        .and_then(Value::as_str)
        .map(ToString::to_string);
    let is_cross_repository = value
        .get("isCrossRepository")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let is_draft = value
        .get("isDraft")
        .and_then(Value::as_bool)
        .unwrap_or(false);

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
        is_archived,
        author_login,
        head_repo_owner,
        head_repo_name,
        is_cross_repository,
        is_draft,
    })
}

/// Parse an issue API URL like `https://api.github.com/repos/<org>/<repo>/issues/123`
/// into `(owner, repo, number)`.
struct IssueRef {
    owner: String,
    repo: String,
    number: u64,
}

fn parse_issue_api_url(api_url: &str) -> Option<IssueRef> {
    let path = api_url
        .strip_prefix("https://api.github.com/repos/")
        .or_else(|| api_url.strip_prefix("repos/"))?;
    let parts: Vec<&str> = path.split('/').collect();
    if parts.len() >= 4 && parts[2] == "issues" {
        let number: u64 = parts[3].parse().ok()?;
        return Some(IssueRef {
            owner: parts[0].to_string(),
            repo: parts[1].to_string(),
            number,
        });
    }
    None
}

fn parse_pr_graphql_value(
    pr_val: &Value,
    owner: &str,
    repo: &str,
    is_archived: bool,
) -> anyhow::Result<PrDetails> {
    let number = pr_val
        .get("number")
        .and_then(Value::as_i64)
        .unwrap_or_default();
    let pr_url = format!("https://github.com/{owner}/{repo}/pull/{number}");
    Ok(PrDetails {
        pr_url,
        owner: owner.to_string(),
        repo: repo.to_string(),
        number,
        state: pr_val
            .get("state")
            .and_then(Value::as_str)
            .unwrap_or("OPEN")
            .to_string(),
        title: pr_val
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("(untitled)")
            .to_string(),
        head_ref: pr_val
            .get("headRefName")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        base_ref: pr_val
            .get("baseRefName")
            .and_then(Value::as_str)
            .unwrap_or("main")
            .to_string(),
        head_sha: pr_val
            .get("headRefOid")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        created_at: pr_val
            .get("createdAt")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        updated_at: pr_val
            .get("updatedAt")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        is_archived,
        author_login: pr_val
            .get("author")
            .and_then(|v| v.get("login"))
            .and_then(Value::as_str)
            .map(ToString::to_string),
        head_repo_owner: pr_val
            .get("headRepositoryOwner")
            .and_then(|v| v.get("login"))
            .and_then(Value::as_str)
            .map(ToString::to_string),
        head_repo_name: pr_val
            .get("headRepository")
            .and_then(|v| v.get("name"))
            .and_then(Value::as_str)
            .map(ToString::to_string),
        is_cross_repository: pr_val
            .get("isCrossRepository")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        is_draft: pr_val
            .get("isDraft")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    })
}

/// Result of a batch GraphQL fetch.
#[derive(Debug, Default)]
pub struct BatchFetchResult {
    pub pr_details: HashMap<String, PrDetails>,
    /// Maps issue API URL to uppercase state string.
    pub issue_states: HashMap<String, String>,
}

/// Fetch details for multiple PRs and issue states in a single GraphQL call.
/// `pr_urls` are HTML PR URLs like `https://github.com/owner/repo/pull/123`.
/// `issue_api_urls` are REST API URLs like `https://api.github.com/repos/o/r/issues/42`.
pub async fn fetch_batch(
    pr_urls: &[String],
    issue_api_urls: &[String],
) -> anyhow::Result<BatchFetchResult> {
    if pr_urls.is_empty() && issue_api_urls.is_empty() {
        return Ok(BatchFetchResult::default());
    }

    let pr_fields = "number title state isDraft headRefName headRefOid baseRefName \
                     createdAt updatedAt author { login } \
                     headRepository { name } headRepositoryOwner { login } \
                     isCrossRepository";

    let mut query = String::from("query {");

    // Deduplicate PRs by (owner, repo, number)
    let mut pr_refs: Vec<(String, String, u64, String)> = Vec::new();
    let mut seen_prs = std::collections::HashSet::new();
    for url in pr_urls {
        if let Ok(parsed) = parse_github_pr_url(url) {
            let key = format!("{}/{}/{}", parsed.owner, parsed.repo, parsed.number);
            if seen_prs.insert(key) {
                pr_refs.push((parsed.owner, parsed.repo, parsed.number, url.clone()));
            }
        }
    }

    for (i, (owner, repo, number, _)) in pr_refs.iter().enumerate() {
        write!(
            query,
            " pr{i}: repository(owner: \"{owner}\", name: \"{repo}\") {{ \
                isArchived pullRequest(number: {number}) {{ {pr_fields} }} \
            }}"
        )?;
    }

    // Deduplicate issues
    let mut issue_refs: Vec<(IssueRef, String)> = Vec::new();
    let mut seen_issues = std::collections::HashSet::new();
    for url in issue_api_urls {
        if let Some(issue) = parse_issue_api_url(url) {
            let key = format!("{}/{}/{}", issue.owner, issue.repo, issue.number);
            if seen_issues.insert(key) {
                issue_refs.push((issue, url.clone()));
            }
        }
    }

    for (i, (issue, _)) in issue_refs.iter().enumerate() {
        write!(
            query,
            " issue{i}: repository(owner: \"{}\", name: \"{}\") {{ \
                issue(number: {}) {{ state }} \
            }}",
            issue.owner, issue.repo, issue.number
        )?;
    }

    query.push('}');

    let output = Cmd::new("gh", ["api", "graphql", "-f", &format!("query={query}")])
        .run()
        .await?;
    output.ensure_success("❌ Failed to run batch GraphQL query")?;

    let response: Value =
        serde_json::from_str(output.stdout()).context("Invalid batch GraphQL JSON")?;
    let data = response.get("data").unwrap_or(&Value::Null);

    let mut result = BatchFetchResult::default();

    for (i, (owner, repo, _, pr_url)) in pr_refs.iter().enumerate() {
        let alias = format!("pr{i}");
        let Some(repo_val) = data.get(&alias) else {
            eprintln!("⚠️ Missing GraphQL alias {alias} for {pr_url}");
            continue;
        };
        if repo_val.is_null() {
            eprintln!("⚠️ GraphQL returned null for {alias} ({pr_url})");
            continue;
        }
        let is_archived = repo_val
            .get("isArchived")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let Some(pr_val) = repo_val.get("pullRequest") else {
            eprintln!("⚠️ No pullRequest in {alias} for {pr_url}");
            continue;
        };
        if pr_val.is_null() {
            eprintln!("⚠️ pullRequest is null for {pr_url}");
            continue;
        }
        match parse_pr_graphql_value(pr_val, owner, repo, is_archived) {
            Ok(details) => {
                result.pr_details.insert(pr_url.clone(), details);
            }
            Err(err) => {
                eprintln!("⚠️ Failed to parse PR details for {pr_url}: {err}");
            }
        }
    }

    for (i, (_, api_url)) in issue_refs.iter().enumerate() {
        let alias = format!("issue{i}");
        if let Some(state) = data
            .get(&alias)
            .and_then(|v| v.get("issue"))
            .and_then(|v| v.get("state"))
            .and_then(Value::as_str)
        {
            result
                .issue_states
                .insert(api_url.clone(), state.to_ascii_uppercase());
        }
    }

    Ok(result)
}

async fn fetch_repository_archived(owner: &str, repo: &str) -> anyhow::Result<bool> {
    let endpoint = format!("/repos/{owner}/{repo}");
    let output = Cmd::new("gh", ["api", &endpoint]).run().await?;
    output.ensure_success(format!(
        "❌ Failed to fetch repository details for {owner}/{repo}"
    ))?;
    anyhow::ensure!(
        !output.stdout().trim().is_empty(),
        "❌ Failed to fetch repository details for {owner}/{repo}: empty output"
    );

    let value: Value =
        serde_json::from_str(output.stdout()).context("Invalid repository details JSON")?;
    Ok(value
        .get("archived")
        .and_then(Value::as_bool)
        .unwrap_or(false))
}

pub async fn mark_notification_done(thread_id: &str) -> anyhow::Result<()> {
    let endpoint = format!("/notifications/threads/{thread_id}");
    let output = Cmd::new("gh", ["api", "-X", "DELETE", &endpoint])
        .run()
        .await?;
    output.ensure_success("❌ Failed to mark notification thread as done")?;
    Ok(())
}

pub async fn ensure_local_repo(owner: &str, repo: &str) -> anyhow::Result<Utf8PathBuf> {
    let repo_dir = local_repo_dir(owner, repo)?;
    ensure_local_repo_at(owner, repo, &repo_dir).await?;
    Ok(repo_dir)
}

pub async fn ensure_local_repo_for_pr(pr_url: &str) -> anyhow::Result<LocalPrRepo> {
    let details = fetch_pr_details(pr_url).await?;
    let clone_target = preferred_clone_target(&details, current_viewer_login().ok().as_deref());
    let repo_dir = local_repo_dir(&clone_target.origin.owner, &clone_target.origin.repo)?;
    ensure_local_repo_at(
        &clone_target.origin.owner,
        &clone_target.origin.repo,
        &repo_dir,
    )
    .await?;

    if let Some(upstream) = clone_target.upstream {
        ensure_remote_repo(&repo_dir, "upstream", &upstream).await?;
    }

    Ok(LocalPrRepo { repo_dir, details })
}

async fn ensure_local_repo_at(owner: &str, repo: &str, repo_dir: &Utf8Path) -> anyhow::Result<()> {
    if fs::try_exists(repo_dir).await? {
        anyhow::ensure!(
            repo_dir.join(".git").exists(),
            "❌ Path exists but is not a git repository: {repo_dir}"
        );
        return Ok(());
    }

    let parent = repo_dir
        .parent()
        .context("Failed to compute repository parent directory")?;
    fs::create_dir_all(parent)
        .await
        .with_context(|| format!("Failed to create {parent}"))?;

    let repo_name = format!("{owner}/{repo}");
    let output = Cmd::new("gh", ["repo", "clone", &repo_name, repo_dir.as_str()])
        .run()
        .await?;
    output.ensure_success(format!("❌ Failed to clone repository {repo_name}"))?;
    Ok(())
}

pub fn local_repo_dir(owner: &str, repo: &str) -> anyhow::Result<Utf8PathBuf> {
    let home = std::env::var("HOME").context("HOME env var is not set")?;
    Ok(Utf8PathBuf::from(home).join("proj").join(owner).join(repo))
}

pub async fn checkout_pr(repo_dir: &Utf8Path, pr_url: &str) -> anyhow::Result<()> {
    let output = Cmd::new("gh", ["pr", "checkout", pr_url])
        .with_current_dir(repo_dir)
        .run()
        .await?;
    output.ensure_success("❌ Failed to checkout PR")?;
    Ok(())
}

pub async fn checkout_pr_for_open_with_details(
    repo_dir: &Utf8Path,
    pr: &PrDetails,
) -> anyhow::Result<()> {
    if current_branch(repo_dir).await.ok().as_deref() == Some(pr.head_ref.as_str()) {
        return Ok(());
    }
    let output = Cmd::new("gh", ["pr", "checkout", pr.pr_url.as_str()])
        .with_current_dir(repo_dir)
        .run()
        .await?;
    if output.status().success() {
        return Ok(());
    }

    if is_diverged_local_branch_error(&output) {
        let detached = Cmd::new("gh", ["pr", "checkout", pr.pr_url.as_str(), "--detach"])
            .with_current_dir(repo_dir)
            .run()
            .await?;
        detached.ensure_success("❌ Failed to checkout PR")?;
        return Ok(());
    }

    output.ensure_success("❌ Failed to checkout PR")?;
    Ok(())
}

pub async fn current_branch(repo_dir: &Utf8Path) -> anyhow::Result<String> {
    let output = Cmd::new("git", ["branch", "--show-current"])
        .with_current_dir(repo_dir)
        .run()
        .await?;
    output.ensure_success("❌ Failed to get current branch")?;
    Ok(output.stdout().to_string())
}

pub async fn is_clean_repo(repo_dir: &Utf8Path) -> anyhow::Result<bool> {
    let output = Cmd::new("git", ["status", "--porcelain"])
        .with_current_dir(repo_dir)
        .run()
        .await?;
    output.ensure_success("❌ Failed to check repository status")?;
    Ok(output.stdout().trim().is_empty())
}

pub async fn default_branch(repo_dir: &Utf8Path) -> anyhow::Result<String> {
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
    .run()
    .await?;

    output.ensure_success("❌ Failed to detect default branch")?;
    anyhow::ensure!(
        !output.stdout().trim().is_empty(),
        "❌ Failed to detect default branch: empty output"
    );

    Ok(output.stdout().to_string())
}

pub async fn checkout_branch(repo_dir: &Utf8Path, branch: &str) -> anyhow::Result<()> {
    let output = Cmd::new("git", ["checkout", branch])
        .with_current_dir(repo_dir)
        .run()
        .await?;
    output.ensure_success(format!("❌ Failed to checkout branch '{branch}'"))?;
    Ok(())
}

pub async fn pull_ff_only(repo_dir: &Utf8Path) -> anyhow::Result<()> {
    let output = Cmd::new("git", ["pull", "--ff-only"])
        .with_current_dir(repo_dir)
        .run()
        .await?;
    output.ensure_success("❌ Failed to pull default branch")?;
    Ok(())
}

fn is_diverged_local_branch_error(output: &CmdOutput) -> bool {
    is_diverged_local_branch_error_text(output.stderr_or_stdout())
}

fn is_diverged_local_branch_error_text(details: &str) -> bool {
    details.contains("Diverging branches can't be fast-forwarded")
        || details.contains("Not possible to fast-forward, aborting.")
}

fn api_url_to_pr_url(api_url: &str, subject_type: Option<&str>) -> Option<String> {
    let trimmed = api_url.trim();

    if let Some(path) = trimmed
        .strip_prefix("https://api.github.com/")
        .unwrap_or(trimmed)
        .strip_prefix("repos/")
    {
        return github_subject_path_to_pr_url(path, subject_type);
    }

    if let Some(path) = trimmed.strip_prefix("https://github.com/") {
        return github_subject_path_to_pr_url(path, subject_type);
    }

    None
}

fn github_subject_path_to_pr_url(path: &str, subject_type: Option<&str>) -> Option<String> {
    let parts: Vec<&str> = path.split('/').collect();
    if parts.len() < 4 {
        return None;
    }

    let is_pull_request = match parts[2] {
        "pulls" | "pull" => true,
        "issues" => matches!(subject_type, Some("PullRequest")),
        _ => false,
    };
    if !is_pull_request {
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

fn current_viewer_login() -> anyhow::Result<String> {
    let output = std::process::Command::new("gh")
        .args(["api", "user", "--jq", ".login"])
        .output()
        .context("❌ Failed to detect current GitHub user")?;
    anyhow::ensure!(
        output.status.success(),
        "❌ Failed to detect current GitHub user"
    );
    let login = String::from_utf8(output.stdout).context("❌ Failed to parse GitHub user login")?;
    anyhow::ensure!(
        !login.trim().is_empty(),
        "❌ Failed to detect current GitHub user: empty output"
    );
    Ok(login.trim().to_string())
}

fn preferred_clone_target(details: &PrDetails, viewer_login: Option<&str>) -> CloneTarget {
    let base_repo = GitHubRepoRef {
        owner: details.owner.clone(),
        repo: details.repo.clone(),
    };

    let clone_from_fork = details.is_cross_repository
        && details.author_login.as_deref() == viewer_login
        && details.head_repo_owner.as_deref() == viewer_login
        && details.head_repo_name.is_some();

    if !clone_from_fork {
        return CloneTarget {
            origin: base_repo,
            upstream: None,
        };
    }

    CloneTarget {
        origin: GitHubRepoRef {
            owner: details.head_repo_owner.clone().unwrap_or_default(),
            repo: details.head_repo_name.clone().unwrap_or_default(),
        },
        upstream: Some(base_repo),
    }
}

async fn ensure_remote_repo(
    repo_dir: &Utf8Path,
    remote_name: &str,
    expected_repo: &GitHubRepoRef,
) -> anyhow::Result<()> {
    let output = Cmd::new("git", ["remote", "get-url", remote_name])
        .with_current_dir(repo_dir)
        .run()
        .await?;

    if output.status().success()
        && parse_github_name_with_owner(output.stdout()).as_deref()
            == Some(format!("{}/{}", expected_repo.owner, expected_repo.repo).as_str())
    {
        return Ok(());
    }

    let expected_url = format!(
        "https://github.com/{}/{}.git",
        expected_repo.owner, expected_repo.repo
    );
    let command = if output.status().success() {
        ["remote", "set-url", remote_name, &expected_url]
    } else {
        ["remote", "add", remote_name, &expected_url]
    };
    let result = Cmd::new("git", command)
        .with_current_dir(repo_dir)
        .run()
        .await?;
    result.ensure_success(format!(
        "❌ Failed to configure {remote_name} remote for {}/{}",
        expected_repo.owner, expected_repo.repo
    ))?;
    Ok(())
}

pub async fn prepare_repo_for_pr_checkout(repo_dir: &Utf8Path) -> anyhow::Result<()> {
    anyhow::ensure!(
        is_clean_repo(repo_dir).await?,
        "❌ Repository is not clean. Commit or stash changes first."
    );
    let default_branch = default_branch(repo_dir).await?;
    Cmd::new("git", ["fetch", "--prune"])
        .with_current_dir(repo_dir)
        .run()
        .await?
        .ensure_success("❌ git fetch failed")?;
    checkout_branch(repo_dir, &default_branch).await?;
    pull_ff_only(repo_dir).await?;
    Ok(())
}

pub fn parse_github_name_with_owner(url: &str) -> Option<String> {
    let trimmed = url.trim().trim_end_matches('/');
    let path = if let Some(path) = trimmed.strip_prefix("git@github.com:") {
        path
    } else if let Some(path) = trimmed.strip_prefix("ssh://git@github.com/") {
        path
    } else if let Some(path) = trimmed.strip_prefix("https://github.com/") {
        path
    } else if let Some(path) = trimmed.strip_prefix("http://github.com/") {
        path
    } else if let Some(path) = trimmed.strip_prefix("git://github.com/") {
        path
    } else {
        return None;
    };

    let normalized = path.strip_suffix(".git").unwrap_or(path).trim_matches('/');
    let mut parts = normalized.split('/');
    let owner = parts.next()?;
    let repo = parts.next()?;
    if parts.next().is_some() || owner.is_empty() || repo.is_empty() {
        return None;
    }

    Some(format!("{owner}/{repo}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_api_pr_url() {
        let pr = api_url_to_pr_url("https://api.github.com/repos/o/r/pulls/123", None).unwrap();
        assert_eq!(pr, "https://github.com/o/r/pull/123");
    }

    #[test]
    fn converts_issue_api_url_for_pull_request_notifications() {
        let pr = api_url_to_pr_url(
            "https://api.github.com/repos/o/r/issues/123",
            Some("PullRequest"),
        )
        .unwrap();
        assert_eq!(pr, "https://github.com/o/r/pull/123");
    }

    #[test]
    fn ignores_issue_api_url_for_non_pull_request_notifications() {
        assert!(
            api_url_to_pr_url("https://api.github.com/repos/o/r/issues/123", Some("Issue"),)
                .is_none()
        );
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

    #[test]
    fn prefers_fork_for_cross_repo_pr_opened_by_viewer() {
        let target = preferred_clone_target(
            &PrDetails {
                pr_url: "https://github.com/upstream/repo/pull/1".to_string(),
                owner: "upstream".to_string(),
                repo: "repo".to_string(),
                number: 1,
                state: "OPEN".to_string(),
                title: "t".to_string(),
                head_ref: "feat".to_string(),
                base_ref: "main".to_string(),
                head_sha: "sha".to_string(),
                created_at: "2026-01-01T00:00:00Z".to_string(),
                updated_at: "2026-01-01T00:00:00Z".to_string(),
                is_archived: false,
                author_login: Some("me".to_string()),
                head_repo_owner: Some("me".to_string()),
                head_repo_name: Some("repo".to_string()),
                is_cross_repository: true,
                is_draft: false,
            },
            Some("me"),
        );

        assert_eq!(
            target,
            CloneTarget {
                origin: GitHubRepoRef {
                    owner: "me".to_string(),
                    repo: "repo".to_string(),
                },
                upstream: Some(GitHubRepoRef {
                    owner: "upstream".to_string(),
                    repo: "repo".to_string(),
                }),
            }
        );
    }

    #[test]
    fn keeps_base_repo_for_non_viewer_prs() {
        let target = preferred_clone_target(
            &PrDetails {
                pr_url: "https://github.com/upstream/repo/pull/1".to_string(),
                owner: "upstream".to_string(),
                repo: "repo".to_string(),
                number: 1,
                state: "OPEN".to_string(),
                title: "t".to_string(),
                head_ref: "feat".to_string(),
                base_ref: "main".to_string(),
                head_sha: "sha".to_string(),
                created_at: "2026-01-01T00:00:00Z".to_string(),
                updated_at: "2026-01-01T00:00:00Z".to_string(),
                is_archived: false,
                author_login: Some("someone-else".to_string()),
                head_repo_owner: Some("someone-else".to_string()),
                head_repo_name: Some("repo".to_string()),
                is_cross_repository: true,
                is_draft: false,
            },
            Some("me"),
        );

        assert_eq!(
            target,
            CloneTarget {
                origin: GitHubRepoRef {
                    owner: "upstream".to_string(),
                    repo: "repo".to_string(),
                },
                upstream: None,
            }
        );
    }

    #[test]
    fn detects_diverged_branch_checkout_error() {
        assert!(is_diverged_local_branch_error_text(
            "Already on 'feature'\nDiverging branches can't be fast-forwarded, you need to either:\nfatal: Not possible to fast-forward, aborting.\n"
        ));
    }

    #[test]
    fn ignores_other_checkout_errors() {
        assert!(!is_diverged_local_branch_error_text(
            "no pull requests found for branch \"feature\"\n"
        ));
    }

    #[test]
    fn test_parse_github_name_with_owner_ssh_style() {
        assert_eq!(
            parse_github_name_with_owner("git@github.com:marcoieni/rust-forge.git"),
            Some("marcoieni/rust-forge".to_string())
        );
    }

    #[test]
    fn test_parse_github_name_with_owner_https_style() {
        assert_eq!(
            parse_github_name_with_owner("https://github.com/marcoieni/rust-forge.git"),
            Some("marcoieni/rust-forge".to_string())
        );
    }

    #[test]
    fn test_parse_github_name_with_owner_rejects_non_github() {
        assert_eq!(
            parse_github_name_with_owner("git@gitlab.com:marcoieni/rust-forge.git"),
            None
        );
    }
}
