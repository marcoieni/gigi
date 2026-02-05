use anyhow::Context as _;
use camino::{Utf8Path, Utf8PathBuf};

use crate::cmd::Cmd;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitHubPrRef {
    pub owner: String,
    pub repo: String,
    pub number: u64,
}

pub fn checkout_pr(pr_url: &str) -> anyhow::Result<()> {
    let pr = parse_github_pr_url(pr_url)?;

    let repo_dir = local_repo_dir(&pr.owner, &pr.repo)?;
    ensure_repo_cloned(&pr.owner, &pr.repo, &repo_dir)?;

    ensure_clean_repo(&repo_dir)?;
    update_default_branch(&repo_dir)?;

    let checkout = Cmd::new("gh", ["pr", "checkout", pr_url])
        .with_title("📥 gh pr checkout ...")
        .with_current_dir(&repo_dir)
        .run();
    anyhow::ensure!(
        checkout.status().success(),
        "❌ Failed to checkout PR: {}",
        checkout.stderr()
    );

    open_vscode(&repo_dir)?;
    Ok(())
}

fn local_repo_dir(owner: &str, repo: &str) -> anyhow::Result<Utf8PathBuf> {
    let home = std::env::var("HOME").context("HOME env var is not set")?;
    Ok(Utf8PathBuf::from(home).join("proj").join(owner).join(repo))
}

fn ensure_repo_cloned(owner: &str, repo: &str, repo_dir: &Utf8Path) -> anyhow::Result<()> {
    if repo_dir.exists() {
        anyhow::ensure!(
            repo_dir.join(".git").exists(),
            "❌ Path exists but is not a git repository: {repo_dir}"
        );
        return Ok(());
    }

    let parent = repo_dir
        .parent()
        .context("Failed to compute parent directory")?;
    std::fs::create_dir_all(parent).with_context(|| format!("Failed to create {parent}"))?;

    let repo_name = format!("{owner}/{repo}");
    let clone = Cmd::new("gh", ["repo", "clone", &repo_name, repo_dir.as_str()])
        .with_title(format!("📦 gh repo clone {repo_name} ..."))
        .run();

    anyhow::ensure!(
        clone.status().success(),
        "❌ Failed to clone repository: {}",
        clone.stderr()
    );

    Ok(())
}

fn ensure_clean_repo(repo_dir: &Utf8Path) -> anyhow::Result<()> {
    let output = Cmd::new("git", ["status", "--porcelain"])
        .with_current_dir(repo_dir)
        .run();
    anyhow::ensure!(
        output.status().success(),
        "❌ Failed to check repository status"
    );
    anyhow::ensure!(
        output.stdout().trim().is_empty(),
        "❌ Repository is not clean. Commit or stash changes first."
    );
    Ok(())
}

fn update_default_branch(repo_dir: &Utf8Path) -> anyhow::Result<()> {
    let default_branch = Cmd::new(
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
    .run();

    anyhow::ensure!(
        default_branch.status().success() && !default_branch.stdout().trim().is_empty(),
        "❌ Failed to detect default branch: {}",
        default_branch.stderr()
    );
    let default_branch = default_branch.stdout().to_string();

    let fetch = Cmd::new("git", ["fetch", "--prune"])
        .with_current_dir(repo_dir)
        .run();
    anyhow::ensure!(fetch.status().success(), "❌ git fetch failed");

    let checkout = Cmd::new("git", ["checkout", &default_branch])
        .with_current_dir(repo_dir)
        .run();
    anyhow::ensure!(
        checkout.status().success(),
        "❌ Failed to checkout default branch '{default_branch}'"
    );

    let pull = Cmd::new("git", ["pull", "--ff-only"])
        .with_current_dir(repo_dir)
        .run();
    anyhow::ensure!(
        pull.status().success(),
        "❌ Failed to pull default branch '{default_branch}'"
    );

    Ok(())
}

fn open_vscode(repo_dir: &Utf8Path) -> anyhow::Result<()> {
    let code = Cmd::new("code", ["."])
        .with_title("🧑‍💻 code .")
        .with_current_dir(repo_dir)
        .run();

    if code.status().success() {
        return Ok(());
    }

    // Fallback for macOS setups where `code` isn't in PATH.
    let open = Cmd::new("open", ["-a", "Visual Studio Code", "."])
        .with_title("🧑‍💻 open -a \"Visual Studio Code\" .")
        .with_current_dir(repo_dir)
        .run();
    anyhow::ensure!(
        open.status().success(),
        "❌ Failed to open VS Code (tried `code .` and `open -a 'Visual Studio Code' .`)"
    );

    Ok(())
}

pub fn parse_github_pr_url(input: &str) -> anyhow::Result<GitHubPrRef> {
    let mut s = input.trim();
    if let Some((before, _)) = s.split_once('#') {
        s = before;
    }
    if let Some((before, _)) = s.split_once('?') {
        s = before;
    }

    s = s
        .strip_prefix("https://")
        .or_else(|| s.strip_prefix("http://"))
        .unwrap_or(s);

    s = s
        .strip_prefix("github.com/")
        .or_else(|| s.strip_prefix("www.github.com/"))
        .context("Expected a github.com PR URL")?;

    let parts: Vec<&str> = s.split('/').filter(|p| !p.is_empty()).collect();
    anyhow::ensure!(
        parts.len() >= 4,
        "Invalid PR URL format (expected /OWNER/REPO/pull/NUMBER)"
    );

    let owner = parts[0];
    let repo = parts[1];
    anyhow::ensure!(parts[2] == "pull", "Invalid PR URL (missing /pull/)");

    let number: u64 = parts[3]
        .parse()
        .with_context(|| format!("Invalid PR number: {}", parts[3]))?;

    Ok(GitHubPrRef {
        owner: owner.to_string(),
        repo: repo.to_string(),
        number,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_basic_url() {
        let pr = parse_github_pr_url("https://github.com/owner/repo/pull/123").unwrap();
        assert_eq!(
            pr,
            GitHubPrRef {
                owner: "owner".to_string(),
                repo: "repo".to_string(),
                number: 123
            }
        );
    }

    #[test]
    fn parse_url_with_trailing_path_and_fragment() {
        let pr = parse_github_pr_url("https://github.com/o/r/pull/42/files#diff").unwrap();
        assert_eq!(pr.owner, "o");
        assert_eq!(pr.repo, "r");
        assert_eq!(pr.number, 42);
    }

    #[test]
    fn parse_url_without_scheme() {
        let pr = parse_github_pr_url("github.com/o/r/pull/1").unwrap();
        assert_eq!(pr.number, 1);
    }

    #[test]
    fn reject_non_pr_url() {
        assert!(parse_github_pr_url("https://github.com/o/r/issues/1").is_err());
    }
}
