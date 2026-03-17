use anyhow::Context as _;

use crate::checkout::parse_github_pr_url;

pub(super) fn api_url_to_pr_url(api_url: &str, subject_type: Option<&str>) -> Option<String> {
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

pub(super) fn api_url_to_html_url(api_url: &str) -> Option<String> {
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

pub(super) fn parse_repo_from_pr_url(pr_url: &str) -> Option<String> {
    let parsed = parse_github_pr_url(pr_url).ok()?;
    Some(format!("{}/{}", parsed.owner, parsed.repo))
}

pub(super) fn current_viewer_login() -> anyhow::Result<String> {
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
    fn converts_api_discussion_url() {
        let discussion =
            api_url_to_html_url("https://api.github.com/repos/o/r/discussions/123").unwrap();
        assert_eq!(discussion, "https://github.com/o/r/discussions/123");
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
