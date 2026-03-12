use anyhow::Context as _;
use camino::{Utf8Path, Utf8PathBuf};
use tokio::fs;

use crate::cmd::{Cmd, CmdOutput};

use super::{
    api::fetch_pr_details,
    parsing::{current_viewer_login, parse_github_name_with_owner},
    types::{CloneTarget, GitHubRepoRef, LocalPrRepo, PrDetails},
};

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

#[cfg(test)]
mod tests {
    use super::*;

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
}
