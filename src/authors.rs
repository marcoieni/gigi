use camino::Utf8Path;

use crate::cmd::Cmd;

pub fn get_co_authors(repo_root: &Utf8Path, default_branch: &str) -> anyhow::Result<Vec<String>> {
    // Get the merge base between current branch and default branch
    let merge_base_output = Cmd::new("git", ["merge-base", "HEAD", default_branch])
        .with_current_dir(repo_root)
        .run();
    anyhow::ensure!(
        merge_base_output.status().success(),
        "Failed to find merge base"
    );
    let merge_base = merge_base_output.stdout().trim();

    // Get all authors from commits in the range
    let authors_output = Cmd::new(
        "git",
        [
            "log",
            "--format=%an <%ae>",
            &format!("{}..HEAD", merge_base),
        ],
    )
    .with_current_dir(repo_root)
    .run();

    anyhow::ensure!(
        authors_output.status().success(),
        "Failed to get commit authors"
    );

    // Get current user to exclude from co-authors
    let current_user_output = Cmd::new("git", ["config", "user.name"])
        .with_current_dir(repo_root)
        .run();
    let current_user_email_output = Cmd::new("git", ["config", "user.email"])
        .with_current_dir(repo_root)
        .run();

    anyhow::ensure!(
        current_user_output.status().success() && current_user_email_output.status().success(),
        "Failed to get current git user name and email"
    );
    let current_author = format!(
        "{} <{}>",
        current_user_output.stdout().trim(),
        current_user_email_output.stdout().trim()
    );

    // Collect unique authors (excluding current user)
    let mut authors = std::collections::HashSet::new();
    for line in authors_output.stdout().lines() {
        let author = line.trim();
        if !author.is_empty() && author != current_author {
            authors.insert(author.to_string());
        }
    }

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

pub fn get_commits_to_squash(repo_root: &Utf8Path, default_branch: &str) -> anyhow::Result<Vec<CommitInfo>> {
    // Get the merge base between current branch and default branch
    let merge_base_output = Cmd::new("git", ["merge-base", "HEAD", default_branch])
        .with_current_dir(repo_root)
        .run();
    anyhow::ensure!(
        merge_base_output.status().success(),
        "Failed to find merge base"
    );
    let merge_base = merge_base_output.stdout().trim();
    
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
    
    anyhow::ensure!(
        commits_output.status().success(),
        "Failed to get commits"
    );
    
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
