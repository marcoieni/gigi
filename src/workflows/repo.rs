use camino::{Utf8Path, Utf8PathBuf};
use serde_json::Value;

use crate::{cmd::Cmd, github};

pub async fn ensure_default_repo_and_root() -> anyhow::Result<Utf8PathBuf> {
    if !is_default_repo_set().await? {
        set_default_repo().await?;
    }
    repo_root().await
}

pub(crate) async fn ensure_clean_repo(repo_root: &Utf8Path) -> anyhow::Result<()> {
    let output = Cmd::new("git", ["status", "--porcelain"])
        .with_current_dir(repo_root)
        .run()
        .await?;
    output.ensure_success("❌ Failed to check repository status")?;
    anyhow::ensure!(
        output.stdout().trim().is_empty(),
        "❌ Repository is not clean. Commit or stash changes first."
    );
    Ok(())
}

struct RepoInfo {
    is_fork: bool,
    default_branch: String,
    parent_name_with_owner: Option<String>,
    parent_default_branch: Option<String>,
}

async fn fetch_repo_info(repo_root: &Utf8Path) -> anyhow::Result<RepoInfo> {
    let origin_repo = origin_name_with_owner(repo_root).await?;
    let output = Cmd::new(
        "gh",
        [
            "repo",
            "view",
            &origin_repo,
            "--json",
            "isFork,parent,defaultBranchRef",
        ],
    )
    .with_current_dir(repo_root)
    .run()
    .await?;
    output.ensure_success("❌ Failed to fetch repository info")?;
    anyhow::ensure!(
        !output.stdout().trim().is_empty(),
        "❌ Failed to fetch repository info: command returned empty output"
    );

    let value: Value = serde_json::from_str(output.stdout())?;
    let is_fork = value
        .get("isFork")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let default_branch = value
        .get("defaultBranchRef")
        .and_then(|value| value.get("name"))
        .and_then(Value::as_str)
        .unwrap_or("main")
        .to_string();

    let (parent_name_with_owner, parent_default_branch) = if is_fork {
        let parent = value.get("parent");
        let name = parent_name_with_owner_from_json(parent);
        let mut branch = parent
            .and_then(|value| value.get("defaultBranchRef"))
            .and_then(|value| value.get("name"))
            .and_then(Value::as_str)
            .map(|branch| branch.to_string());

        if branch.is_none()
            && let Some(parent_repo) = name.as_deref()
        {
            branch = Some(fetch_default_branch(repo_root, parent_repo).await?);
        }
        (name, branch)
    } else {
        (None, None)
    };

    Ok(RepoInfo {
        is_fork,
        default_branch,
        parent_name_with_owner,
        parent_default_branch,
    })
}

fn parent_name_with_owner_from_json(parent: Option<&Value>) -> Option<String> {
    parent.and_then(|value| {
        value
            .get("nameWithOwner")
            .and_then(Value::as_str)
            .map(|name| name.to_string())
            .or_else(|| {
                let owner = value
                    .get("owner")
                    .and_then(|owner| owner.get("login"))
                    .and_then(Value::as_str)?;
                let name = value.get("name").and_then(Value::as_str)?;
                Some(format!("{owner}/{name}"))
            })
    })
}

async fn fetch_default_branch(
    repo_root: &Utf8Path,
    repo_name_with_owner: &str,
) -> anyhow::Result<String> {
    let output = Cmd::new(
        "gh",
        [
            "repo",
            "view",
            repo_name_with_owner,
            "--json",
            "defaultBranchRef",
        ],
    )
    .with_current_dir(repo_root)
    .run()
    .await?;
    output.ensure_success("❌ Failed to fetch repository default branch")?;
    anyhow::ensure!(
        !output.stdout().trim().is_empty(),
        "❌ Failed to fetch repository default branch: command returned empty output"
    );

    let value: Value = serde_json::from_str(output.stdout())?;
    Ok(value
        .get("defaultBranchRef")
        .and_then(|branch| branch.get("name"))
        .and_then(Value::as_str)
        .unwrap_or("main")
        .to_string())
}

async fn origin_name_with_owner(repo_root: &Utf8Path) -> anyhow::Result<String> {
    let output = Cmd::new("git", ["remote", "get-url", "origin"])
        .with_current_dir(repo_root)
        .run()
        .await?;
    output.ensure_success("❌ Failed to detect origin remote URL")?;
    anyhow::ensure!(
        !output.stdout().trim().is_empty(),
        "❌ Failed to detect origin remote URL: command returned empty output"
    );

    github::parse_github_name_with_owner(output.stdout()).ok_or_else(|| {
        anyhow::anyhow!(
            "❌ Failed to parse origin remote URL as GitHub repository: {}",
            output.stdout()
        )
    })
}

async fn ensure_upstream_remote(
    repo_root: &Utf8Path,
    parent_name_with_owner: &str,
) -> anyhow::Result<()> {
    let output = Cmd::new("git", ["remote", "get-url", "upstream"])
        .with_current_dir(repo_root)
        .run()
        .await?;
    if output.status().success() && !output.stdout().trim().is_empty() {
        return Ok(());
    }

    let upstream_url = format!("https://github.com/{parent_name_with_owner}.git");
    let add_output = Cmd::new("git", ["remote", "add", "upstream", &upstream_url])
        .with_current_dir(repo_root)
        .run()
        .await?;
    add_output.ensure_success("❌ Failed to add upstream remote")?;
    Ok(())
}

pub async fn sync_fork(repo_root: &Utf8Path) -> anyhow::Result<()> {
    ensure_clean_repo(repo_root).await?;

    let repo_info = fetch_repo_info(repo_root).await?;
    if !repo_info.is_fork {
        println!("ℹ️ Repository is not a fork. Nothing to sync.");
        return Ok(());
    }

    let parent_name_with_owner = repo_info
        .parent_name_with_owner
        .ok_or_else(|| anyhow::anyhow!("❌ Failed to detect parent repository"))?;
    let parent_default_branch = repo_info
        .parent_default_branch
        .ok_or_else(|| anyhow::anyhow!("❌ Failed to detect parent default branch"))?;

    ensure_upstream_remote(repo_root, &parent_name_with_owner).await?;

    let current = current_branch(repo_root).await?;
    Cmd::new("git", ["fetch", "upstream"])
        .with_current_dir(repo_root)
        .run()
        .await?
        .ensure_success("❌ Failed to fetch upstream remote")?;
    Cmd::new("git", ["checkout", &repo_info.default_branch])
        .with_current_dir(repo_root)
        .run()
        .await?
        .ensure_success(format!(
            "❌ Failed to checkout default branch '{}'",
            repo_info.default_branch
        ))?;
    let pull_output = Cmd::new(
        "git",
        ["pull", "--ff-only", "upstream", &parent_default_branch],
    )
    .with_current_dir(repo_root)
    .run()
    .await?;
    pull_output.ensure_success("❌ Failed to sync default branch from upstream")?;
    let push_output = Cmd::new("git", ["push", "origin", &repo_info.default_branch])
        .with_current_dir(repo_root)
        .run()
        .await?;
    push_output.ensure_success("❌ Failed to push synced default branch to origin")?;

    if current != repo_info.default_branch {
        Cmd::new("git", ["checkout", &current])
            .with_current_dir(repo_root)
            .run()
            .await?
            .ensure_success(format!("❌ Failed to switch back to branch '{current}'"))?;
    }

    Ok(())
}

async fn is_default_repo_set() -> anyhow::Result<bool> {
    let output = Cmd::new("gh", ["repo", "set-default", "--view"])
        .hide_stdout()
        .hide_stderr()
        .run()
        .await?;
    Ok(!output.stdout().trim().is_empty())
}

async fn set_default_repo() -> anyhow::Result<()> {
    let remotes_output = Cmd::new("git", ["remote"]).hide_stdout().run().await?;
    let has_upstream = remotes_output
        .stdout()
        .lines()
        .any(|line| line.trim() == "upstream");

    let remote = if has_upstream { "upstream" } else { "origin" };
    Cmd::new("gh", ["repo", "set-default", remote])
        .run()
        .await?
        .ensure_success(format!(
            "❌ Failed to set default GitHub repo to remote '{remote}'"
        ))?;
    Ok(())
}

pub(crate) async fn default_branch(repo_root: &Utf8Path) -> anyhow::Result<String> {
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
    .with_current_dir(repo_root)
    .run()
    .await?;
    output.ensure_success("❌ Failed to detect default branch")?;
    anyhow::ensure!(
        !output.stdout().trim().is_empty(),
        "❌ Failed to detect default branch: command returned empty output"
    );
    Ok(output.stdout().to_string())
}

pub(crate) async fn current_branch(repo_root: &Utf8Path) -> anyhow::Result<String> {
    let output = Cmd::new("git", ["branch", "--show-current"])
        .with_current_dir(repo_root)
        .run()
        .await?;
    output.ensure_success("❌ Failed to detect current branch")?;
    anyhow::ensure!(
        !output.stdout().trim().is_empty(),
        "❌ Failed to detect current branch: command returned empty output"
    );
    Ok(output.stdout().to_string())
}

pub(crate) async fn ensure_not_on_default_branch(
    repo_root: &Utf8Path,
    default_branch: &str,
) -> anyhow::Result<()> {
    let current_branch = current_branch(repo_root).await?;
    anyhow::ensure!(
        current_branch != default_branch,
        "❌ Cannot push to default branch '{default_branch}'. Switch to a feature branch first."
    );
    Ok(())
}

pub(crate) async fn commit(repo_root: &Utf8Path, commit_message: &str) -> anyhow::Result<()> {
    let output = Cmd::new("git", ["commit", "-m", commit_message])
        .with_current_dir(repo_root)
        .run()
        .await?;
    output.ensure_success("❌ git commit failed")?;

    if output.stdout().contains("nothing to commit") {
        anyhow::bail!("❌ Nothing to commit");
    }
    Ok(())
}

pub(crate) async fn view_pr_in_browser(repo_root: &Utf8Path) -> anyhow::Result<()> {
    Cmd::new("gh", ["pr", "view", "--web"])
        .with_current_dir(repo_root)
        .run()
        .await?
        .ensure_success("❌ Failed to open PR in browser")?;
    Ok(())
}

pub(crate) async fn repo_root() -> anyhow::Result<Utf8PathBuf> {
    let git_root = Cmd::new("git", ["rev-parse", "--show-toplevel"])
        .hide_stdout()
        .run()
        .await?;
    git_root.ensure_success("❌ Failed to resolve git repository root")?;
    anyhow::ensure!(
        !git_root.stdout().trim().is_empty(),
        "❌ Failed to resolve git repository root: command returned empty output"
    );
    Ok(Utf8PathBuf::from(git_root.stdout()))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::parent_name_with_owner_from_json;

    #[test]
    fn test_parent_name_with_owner_from_json_name_with_owner_field() {
        let parent = json!({"nameWithOwner": "rust-lang/rust-forge"});
        assert_eq!(
            parent_name_with_owner_from_json(Some(&parent)),
            Some("rust-lang/rust-forge".to_string())
        );
    }

    #[test]
    fn test_parent_name_with_owner_from_json_owner_name_fallback() {
        let parent = json!({
            "name": "rust-forge",
            "owner": { "login": "rust-lang" }
        });
        assert_eq!(
            parent_name_with_owner_from_json(Some(&parent)),
            Some("rust-lang/rust-forge".to_string())
        );
    }
}
