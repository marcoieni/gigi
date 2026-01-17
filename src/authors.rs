use std::collections::HashSet;

use camino::Utf8Path;

use crate::cmd::Cmd;

/// Extract email from author string "Name <email>"
fn extract_email(author: &str) -> Option<String> {
    if let Some(start) = author.rfind('<')
        && let Some(end) = author.rfind('>')
        && start < end
    {
        return Some(author[start + 1..end].trim().to_string());
    }
    None
}

/// Get all authors from commits in the range
fn get_commit_authors(repo_root: &Utf8Path, merge_base: &str) -> anyhow::Result<String> {
    let output = Cmd::new(
        "git",
        [
            "log",
            "--format=%an <%ae>",
            &format!("{}..HEAD", merge_base),
        ],
    )
    .with_current_dir(repo_root)
    .run();
    anyhow::ensure!(output.status().success(), "Failed to get commit authors");
    Ok(output.stdout().to_string())
}

fn get_commit_messages(repo_root: &Utf8Path, merge_base: &str) -> anyhow::Result<String> {
    let output = Cmd::new(
        "git",
        ["log", "--format=%B", &format!("{}..HEAD", merge_base)],
    )
    .with_current_dir(repo_root)
    .run();
    anyhow::ensure!(output.status().success(), "Failed to get commit messages");
    Ok(output.stdout().to_string())
}

fn get_current_user_email(repo_root: &Utf8Path) -> anyhow::Result<String> {
    let output = Cmd::new("git", ["config", "user.email"])
        .with_current_dir(repo_root)
        .run();
    anyhow::ensure!(
        output.status().success(),
        "Failed to get current git user email"
    );
    Ok(output.stdout().trim().to_string())
}

fn collect_authors_from_log(authors_output: &str, current_user_email: &str) -> HashSet<String> {
    let mut authors = HashSet::new();
    for line in authors_output.lines() {
        let author = line.trim();
        if !author.is_empty()
            && let Some(email) = extract_email(author)
            && email.to_lowercase() != current_user_email.to_lowercase()
        {
            authors.insert(author.to_string());
        }
    }
    authors
}

fn parse_co_authors_from_messages(
    commit_messages: &str,
    current_user_email: &str,
    authors: &mut HashSet<String>,
) {
    for line in commit_messages.lines() {
        let line = line.trim();
        if line.starts_with("Co-authored-by:")
            && let Some(co_author) = line.strip_prefix("Co-authored-by:").map(|s| s.trim())
            && !co_author.is_empty()
            && let Some(email) = extract_email(co_author)
            && email.to_lowercase() != current_user_email.to_lowercase()
        {
            authors.insert(co_author.to_string());
        }
    }
}

pub fn get_co_authors(repo_root: &Utf8Path, merge_base: &str) -> anyhow::Result<Vec<String>> {
    let authors_output = get_commit_authors(repo_root, merge_base)?;
    let commit_messages = get_commit_messages(repo_root, merge_base)?;
    let current_user_email = get_current_user_email(repo_root)?;

    let mut authors = collect_authors_from_log(&authors_output, &current_user_email);
    parse_co_authors_from_messages(&commit_messages, &current_user_email, &mut authors);

    Ok(authors.into_iter().collect())
}

pub fn format_co_authors(co_authors: &[String]) -> String {
    let result = if co_authors.is_empty() {
        String::new()
    } else {
        let mut result = String::new();
        for author in co_authors {
            result.push_str(&format!("\nCo-authored-by: {}", author));
        }
        result
    };
    if result.is_empty() {
        String::new()
    } else {
        format!("\n{result}")
    }
}

#[derive(Debug)]
pub struct CommitInfo {
    pub hash: String,
    pub message: String,
    pub author: String,
}

pub fn get_commits_to_squash(
    repo_root: &Utf8Path,
    merge_base: &str,
) -> anyhow::Result<Vec<CommitInfo>> {
    // Get commits with hash, subject, and author
    let commits_output = Cmd::new(
        "git",
        [
            "log",
            "--format=%H|%s|%an <%ae>",
            &format!("{}..HEAD", merge_base),
        ],
    )
    .with_current_dir(repo_root)
    .run();

    anyhow::ensure!(commits_output.status().success(), "Failed to get commits");

    let mut commits = Vec::new();
    for line in commits_output.stdout().lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let parts: Vec<&str> = line.splitn(3, '|').collect();
        if parts.len() == 3 {
            commits.push(CommitInfo {
                hash: parts[0][..8].to_string(), // Abbreviated hash
                message: parts[1].to_string(),
                author: parts[2].to_string(),
            });
        }
    }

    // Reverse to show commits in chronological order
    commits.reverse();
    Ok(commits)
}
