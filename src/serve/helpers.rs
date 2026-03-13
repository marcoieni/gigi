use camino::Utf8PathBuf;

use crate::{config::AppConfig, github};

pub(crate) fn parse_repository_name(repository: &str) -> anyhow::Result<(String, String)> {
    let repository = repository.trim();
    let Some((owner, repo)) = repository.split_once('/') else {
        anyhow::bail!("Invalid repository name '{repository}' (expected owner/repo)");
    };
    let owner = owner.trim();
    let repo = repo.trim();
    anyhow::ensure!(
        !owner.is_empty()
            && !repo.is_empty()
            && !repo.contains('/')
            && !owner.chars().any(char::is_whitespace)
            && !repo.chars().any(char::is_whitespace),
        "Invalid repository name '{repository}' (expected owner/repo)"
    );
    Ok((owner.to_string(), repo.to_string()))
}

pub(crate) async fn resolve_open_target_repo(
    repository: &str,
    pr_url: Option<&str>,
) -> anyhow::Result<Utf8PathBuf> {
    if let Some(pr_url) = pr_url {
        let local_pr = github::ensure_local_repo_for_pr(pr_url).await?;
        println!(
            "🔀 Preparing PR for open action: {}",
            local_pr.details.pr_url
        );
        github::checkout_pr_for_open_with_details(&local_pr.repo_dir, &local_pr.details).await?;
        return Ok(local_pr.repo_dir);
    }

    let (owner, repo) = parse_repository_name(repository)?;
    github::ensure_local_repo(&owner, &repo).await
}

pub(crate) fn describe_open_target(repository: &str, pr_url: Option<&str>) -> String {
    match pr_url {
        Some(pr_url) => format!("{repository} ({pr_url})"),
        None => repository.to_string(),
    }
}

pub(crate) fn dashboard_browser_url(config: &AppConfig) -> String {
    let host = match config.dashboard.host.trim() {
        "0.0.0.0" | "::" => "localhost".to_string(),
        other if other.contains(':') && !other.starts_with('[') && !other.ends_with(']') => {
            format!("[{other}]")
        }
        other => other.to_string(),
    };
    format!("http://{host}:{}", config.dashboard.port)
}
