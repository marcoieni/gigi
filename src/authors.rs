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
        ["log", "--format=%an <%ae>", &format!("{merge_base}..HEAD")],
    )
    .with_current_dir(repo_root)
    .run();
    anyhow::ensure!(output.status().success(), "Failed to get commit authors");
    Ok(output.stdout().to_string())
}

fn get_commit_messages(repo_root: &Utf8Path, merge_base: &str) -> anyhow::Result<String> {
    let output = Cmd::new(
        "git",
        ["log", "--format=%B", &format!("{merge_base}..HEAD")],
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
            result.push_str(&format!("\nCo-authored-by: {author}"));
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
            &format!("{merge_base}..HEAD"),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_email_valid() {
        assert_eq!(
            extract_email("John Doe <john@example.com>"),
            Some("john@example.com".to_string())
        );
    }

    #[test]
    fn test_extract_email_with_spaces() {
        assert_eq!(
            extract_email("John Doe < john@example.com >"),
            Some("john@example.com".to_string())
        );
    }

    #[test]
    fn test_extract_email_no_brackets() {
        assert_eq!(extract_email("john@example.com"), None);
    }

    #[test]
    fn test_extract_email_empty() {
        assert_eq!(extract_email(""), None);
    }

    #[test]
    fn test_extract_email_malformed() {
        assert_eq!(extract_email("John Doe >john@example.com<"), None);
    }

    #[test]
    fn test_collect_authors_from_log_filters_current_user() {
        let log = "Alice <alice@example.com>\nBob <bob@example.com>\nAlice <alice@example.com>";
        let authors = collect_authors_from_log(log, "alice@example.com");
        assert_eq!(authors.len(), 1);
        assert!(authors.contains("Bob <bob@example.com>"));
    }

    #[test]
    fn test_collect_authors_from_log_case_insensitive_email() {
        let log = "Alice <ALICE@EXAMPLE.COM>";
        let authors = collect_authors_from_log(log, "alice@example.com");
        assert!(authors.is_empty());
    }

    #[test]
    fn test_collect_authors_from_log_skips_empty_lines() {
        let log = "\n\nAlice <alice@example.com>\n\n";
        let authors = collect_authors_from_log(log, "bob@example.com");
        assert_eq!(authors.len(), 1);
    }

    #[test]
    fn test_parse_co_authors_from_messages() {
        let messages = "Some commit message\nCo-authored-by: Carol <carol@example.com>\n";
        let mut authors = HashSet::new();
        parse_co_authors_from_messages(messages, "bob@example.com", &mut authors);
        assert_eq!(authors.len(), 1);
        assert!(authors.contains("Carol <carol@example.com>"));
    }

    #[test]
    fn test_parse_co_authors_filters_current_user() {
        let messages = "Co-authored-by: Bob <bob@example.com>";
        let mut authors = HashSet::new();
        parse_co_authors_from_messages(messages, "bob@example.com", &mut authors);
        assert!(authors.is_empty());
    }

    #[test]
    fn test_parse_co_authors_with_whitespace() {
        let messages = "  Co-authored-by:   Dave <dave@example.com>  ";
        let mut authors = HashSet::new();
        parse_co_authors_from_messages(messages, "other@example.com", &mut authors);
        assert_eq!(authors.len(), 1);
    }

    #[test]
    fn test_format_co_authors_empty() {
        assert_eq!(format_co_authors(&[]), "");
    }

    #[test]
    fn test_format_co_authors_single() {
        let result = format_co_authors(&["Alice <alice@example.com>".to_string()]);
        assert_eq!(result, "\n\nCo-authored-by: Alice <alice@example.com>");
    }

    #[test]
    fn test_format_co_authors_multiple() {
        let result = format_co_authors(&[
            "Alice <alice@example.com>".to_string(),
            "Bob <bob@example.com>".to_string(),
        ]);
        assert!(result.contains("Co-authored-by: Alice <alice@example.com>"));
        assert!(result.contains("Co-authored-by: Bob <bob@example.com>"));
    }
}
