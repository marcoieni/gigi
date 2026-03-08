use anyhow::Context as _;

use crate::{github, launcher};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitHubPrRef {
    pub owner: String,
    pub repo: String,
    pub number: u64,
}

pub async fn checkout_pr(pr_url: &str) -> anyhow::Result<()> {
    let local_pr = github::ensure_local_repo_for_pr(pr_url).await?;
    github::prepare_repo_for_pr_checkout(&local_pr.repo_dir).await?;
    github::checkout_pr(&local_pr.repo_dir, pr_url).await?;
    launcher::open_vscode(&local_pr.repo_dir).await?;
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
