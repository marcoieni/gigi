use std::{
    collections::{HashMap, HashSet},
    fmt,
};

use camino::Utf8Path;
use chrono::{DateTime, Local, Utc};
use inquire::MultiSelect;

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
async fn get_commit_authors(repo_root: &Utf8Path, merge_base: &str) -> anyhow::Result<String> {
    let output = Cmd::new(
        "git",
        ["log", "--format=%an <%ae>", &format!("{merge_base}..HEAD")],
    )
    .with_current_dir(repo_root)
    .run()
    .await?;
    output.ensure_success("Failed to get commit authors")?;
    Ok(output.stdout().to_string())
}

async fn get_commit_messages(repo_root: &Utf8Path, merge_base: &str) -> anyhow::Result<String> {
    let output = Cmd::new(
        "git",
        ["log", "--format=%B", &format!("{merge_base}..HEAD")],
    )
    .with_current_dir(repo_root)
    .run()
    .await?;
    output.ensure_success("Failed to get commit messages")?;
    Ok(output.stdout().to_string())
}

async fn get_current_user_email(repo_root: &Utf8Path) -> anyhow::Result<String> {
    let output = Cmd::new("git", ["config", "user.email"])
        .with_current_dir(repo_root)
        .run()
        .await?;
    output.ensure_success("Failed to get current git user email")?;
    Ok(output.stdout().trim().to_string())
}

fn collect_authors_from_log(authors_output: &str, current_user_email: &str) -> HashSet<String> {
    let mut authors = HashSet::new();
    for line in authors_output.lines() {
        let author = line.trim();
        if !author.is_empty()
            && let Some(email) = extract_email(author)
            && !email.eq_ignore_ascii_case(current_user_email)
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
            && !email.eq_ignore_ascii_case(current_user_email)
        {
            authors.insert(co_author.to_string());
        }
    }
}

fn sort_authors(authors: &mut [String]) {
    authors.sort_unstable_by_key(|author| author.to_ascii_lowercase());
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SelectableCoAuthor {
    author: String,
    last_committed_at: i64,
}

impl SelectableCoAuthor {
    fn new(author: String, last_committed_at: i64) -> Self {
        Self {
            author,
            last_committed_at,
        }
    }
}

impl fmt::Display for SelectableCoAuthor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Some(last_committed_at) = DateTime::<Utc>::from_timestamp(self.last_committed_at, 0)
        else {
            return write!(f, "{} (last commit: unknown)", self.author);
        };

        let last_committed_at = last_committed_at
            .with_timezone(&Local)
            .format("%Y-%m-%d %H:%M %:z");
        write!(f, "{} (last commit: {last_committed_at})", self.author)
    }
}

pub async fn get_co_authors(repo_root: &Utf8Path, merge_base: &str) -> anyhow::Result<Vec<String>> {
    let authors_output = get_commit_authors(repo_root, merge_base).await?;
    let commit_messages = get_commit_messages(repo_root, merge_base).await?;
    let current_user_email = get_current_user_email(repo_root).await?;

    let mut authors = collect_authors_from_log(&authors_output, &current_user_email);
    parse_co_authors_from_messages(&commit_messages, &current_user_email, &mut authors);

    let mut authors: Vec<_> = authors.into_iter().collect();
    sort_authors(&mut authors);
    Ok(authors)
}

fn collect_excluded_emails(
    existing_authors: &[String],
    current_user_email: &str,
) -> HashSet<String> {
    let mut excluded_emails = HashSet::from([current_user_email.to_ascii_lowercase()]);
    for author in existing_authors {
        if let Some(email) = extract_email(author) {
            excluded_emails.insert(email.to_ascii_lowercase());
        }
    }
    excluded_emails
}

fn parse_author_history_line(
    line: &str,
    excluded_emails: &HashSet<String>,
) -> Option<(String, i64)> {
    let trimmed = line.trim();
    let (timestamp, author) = trimmed.split_once('|')?;
    let author = author.trim();
    let timestamp = timestamp.parse::<i64>().ok()?;
    let email = extract_email(author)?;
    if excluded_emails.contains(&email.to_ascii_lowercase()) {
        None
    } else {
        Some((author.to_string(), timestamp))
    }
}

fn collect_selectable_co_authors(
    author_history_output: &str,
    current_user_email: &str,
    existing_authors: &[String],
) -> Vec<SelectableCoAuthor> {
    let excluded_emails = collect_excluded_emails(existing_authors, current_user_email);
    let mut authors: HashMap<String, i64> = HashMap::new();

    for line in author_history_output.lines() {
        if let Some((author, timestamp)) = parse_author_history_line(line, &excluded_emails) {
            authors
                .entry(author)
                .and_modify(|latest_timestamp| {
                    *latest_timestamp = (*latest_timestamp).max(timestamp);
                })
                .or_insert(timestamp);
        }
    }

    let mut authors: Vec<_> = authors
        .into_iter()
        .map(|(author, last_committed_at)| SelectableCoAuthor::new(author, last_committed_at))
        .collect();
    authors.sort_unstable_by(|left, right| {
        right
            .last_committed_at
            .cmp(&left.last_committed_at)
            .then_with(|| {
                left.author
                    .to_ascii_lowercase()
                    .cmp(&right.author.to_ascii_lowercase())
            })
    });
    authors
}

pub async fn get_selectable_co_authors(
    repo_root: &Utf8Path,
    existing_authors: &[String],
) -> anyhow::Result<Vec<SelectableCoAuthor>> {
    let current_user_email = get_current_user_email(repo_root).await?;
    let output = Cmd::new("git", ["log", "--all", "--format=%ct|%an <%ae>"])
        .with_current_dir(repo_root)
        .run()
        .await?;
    output.ensure_success("Failed to get repository author history")?;

    Ok(collect_selectable_co_authors(
        output.stdout(),
        &current_user_email,
        existing_authors,
    ))
}

pub fn prompt_for_additional_co_authors(
    candidates: &[SelectableCoAuthor],
) -> anyhow::Result<Vec<String>> {
    if candidates.is_empty() {
        return Ok(Vec::new());
    }

    let selected =
        MultiSelect::new("Select additional co-authors:", candidates.to_vec()).prompt()?;
    Ok(selected
        .into_iter()
        .map(|candidate| candidate.author)
        .collect())
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

pub async fn get_commits_to_squash(
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
    .run()
    .await?;

    commits_output.ensure_success("Failed to get commits")?;

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

    #[test]
    fn test_collect_selectable_co_authors_filters_current_and_existing() {
        let author_history = "\
            1713000000|Alice <alice@example.com>\n\
            1712900000|Bob <bob@example.com>\n\
            1712800000|Carol <carol@example.com>\n";

        let authors = collect_selectable_co_authors(
            author_history,
            "alice@example.com",
            &["Bob <bob@example.com>".to_string()],
        );

        assert_eq!(
            authors,
            vec![SelectableCoAuthor::new(
                "Carol <carol@example.com>".to_string(),
                1_712_800_000
            )]
        );
    }

    #[test]
    fn test_collect_selectable_co_authors_deduplicates_and_sorts_by_recency() {
        let author_history = "\
            1713000000|Alice <alice@example.com>\n\
            1713200000|Bob <bob@example.com>\n\
            1713100000|Bob <bob@example.com>\n\
            1713300000|Carol <carol@example.com>\n";

        let authors = collect_selectable_co_authors(author_history, "me@example.com", &[]);

        assert_eq!(
            authors,
            vec![
                SelectableCoAuthor::new("Carol <carol@example.com>".to_string(), 1_713_300_000),
                SelectableCoAuthor::new("Bob <bob@example.com>".to_string(), 1_713_200_000),
                SelectableCoAuthor::new("Alice <alice@example.com>".to_string(), 1_713_000_000)
            ]
        );
    }
}
