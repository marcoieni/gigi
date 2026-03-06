use anyhow::Context as _;
use camino::{Utf8Path, Utf8PathBuf};
use serde_json::Value;

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
            "title,url,state,headRefName,headRefOid,baseRefName,createdAt,updatedAt,number,author,headRepository,headRepositoryOwner,isCrossRepository",
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
    let is_archived = fetch_repository_archived(&parsed.owner, &parsed.repo)?;
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
    })
}

fn fetch_repository_archived(owner: &str, repo: &str) -> anyhow::Result<bool> {
    let endpoint = format!("/repos/{owner}/{repo}");
    let output = Cmd::new("gh", ["api", &endpoint]).run()?;
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

pub fn mark_notification_done(thread_id: &str) -> anyhow::Result<()> {
    let endpoint = format!("/notifications/threads/{thread_id}");
    let output = Cmd::new("gh", ["api", "-X", "DELETE", &endpoint]).run()?;
    output.ensure_success("❌ Failed to mark notification thread as done")?;
    Ok(())
}

pub fn ensure_local_repo(owner: &str, repo: &str) -> anyhow::Result<Utf8PathBuf> {
    let repo_dir = local_repo_dir(owner, repo)?;
    ensure_local_repo_at(owner, repo, &repo_dir)?;
    Ok(repo_dir)
}

pub fn ensure_local_repo_for_pr(pr_url: &str) -> anyhow::Result<LocalPrRepo> {
    let details = fetch_pr_details(pr_url)?;
    let clone_target = preferred_clone_target(&details, current_viewer_login().ok().as_deref());
    let repo_dir = local_repo_dir(&clone_target.origin.owner, &clone_target.origin.repo)?;
    ensure_local_repo_at(
        &clone_target.origin.owner,
        &clone_target.origin.repo,
        &repo_dir,
    )?;

    if let Some(upstream) = clone_target.upstream {
        ensure_remote_repo(&repo_dir, "upstream", &upstream)?;
    }

    Ok(LocalPrRepo { repo_dir, details })
}

fn ensure_local_repo_at(owner: &str, repo: &str, repo_dir: &Utf8Path) -> anyhow::Result<()> {
    if repo_dir.exists() {
        anyhow::ensure!(
            repo_dir.join(".git").exists(),
            "❌ Path exists but is not a git repository: {repo_dir}"
        );
        return Ok(());
    }

    let parent = repo_dir
        .parent()
        .context("Failed to compute repository parent directory")?;
    std::fs::create_dir_all(parent).with_context(|| format!("Failed to create {parent}"))?;

    let repo_name = format!("{owner}/{repo}");
    let output = Cmd::new("gh", ["repo", "clone", &repo_name, repo_dir.as_str()]).run()?;
    output.ensure_success(format!("❌ Failed to clone repository {repo_name}"))?;
    Ok(())
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

pub fn checkout_pr_for_open_with_details(
    repo_dir: &Utf8Path,
    pr: &PrDetails,
) -> anyhow::Result<()> {
    if current_branch(repo_dir).ok().as_deref() == Some(pr.head_ref.as_str()) {
        return Ok(());
    }
    let output = Cmd::new("gh", ["pr", "checkout", pr.pr_url.as_str()])
        .with_current_dir(repo_dir)
        .run()?;
    if output.status().success() {
        return Ok(());
    }

    if is_diverged_local_branch_error(&output) {
        let detached = Cmd::new("gh", ["pr", "checkout", pr.pr_url.as_str(), "--detach"])
            .with_current_dir(repo_dir)
            .run()?;
        detached.ensure_success("❌ Failed to checkout PR")?;
        return Ok(());
    }

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

fn is_diverged_local_branch_error(output: &CmdOutput) -> bool {
    is_diverged_local_branch_error_text(output.stderr_or_stdout())
}

fn is_diverged_local_branch_error_text(details: &str) -> bool {
    details.contains("Diverging branches can't be fast-forwarded")
        || details.contains("Not possible to fast-forward, aborting.")
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

fn current_viewer_login() -> anyhow::Result<String> {
    let output = Cmd::new("gh", ["api", "user", "--jq", ".login"]).run()?;
    output.ensure_success("❌ Failed to detect current GitHub user")?;
    anyhow::ensure!(
        !output.stdout().trim().is_empty(),
        "❌ Failed to detect current GitHub user: empty output"
    );
    Ok(output.stdout().to_string())
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

fn ensure_remote_repo(
    repo_dir: &Utf8Path,
    remote_name: &str,
    expected_repo: &GitHubRepoRef,
) -> anyhow::Result<()> {
    let output = Cmd::new("git", ["remote", "get-url", remote_name])
        .with_current_dir(repo_dir)
        .run()?;

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
    let result = Cmd::new("git", command).with_current_dir(repo_dir).run()?;
    result.ensure_success(format!(
        "❌ Failed to configure {remote_name} remote for {}/{}",
        expected_repo.owner, expected_repo.repo
    ))?;
    Ok(())
}

fn parse_github_name_with_owner(url: &str) -> Option<String> {
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
}
