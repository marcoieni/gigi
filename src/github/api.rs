use std::{collections::HashMap, fmt::Write as _};

use anyhow::Context as _;
use serde_json::Value;

use crate::{checkout::parse_github_pr_url, cmd::Cmd};

use super::{
    parsing::{api_url_to_html_url, api_url_to_pr_url, parse_repo_from_pr_url},
    types::{AuthoredPrSummary, BatchFetchResult, NotificationThread, Participant, PrDetails},
};

pub async fn fetch_notifications(since: Option<&str>) -> anyhow::Result<Vec<NotificationThread>> {
    let endpoint = notifications_endpoint(since);
    let output = Cmd::new("gh", ["api", &endpoint, "--paginate", "--slurp"])
        .run()
        .await?;
    output.ensure_success("❌ Failed to fetch notifications")?;

    if output.stdout().trim().is_empty() {
        return Ok(Vec::new());
    }

    let value: Value =
        serde_json::from_str(output.stdout()).context("Invalid notifications JSON")?;

    let pages: Vec<Value> = match value {
        Value::Array(items) if items.iter().all(Value::is_array) => items,
        Value::Array(items) => vec![Value::Array(items)],
        _ => vec![],
    };

    let mut results = Vec::new();
    for page in pages {
        let Value::Array(entries) = page else {
            continue;
        };

        for entry in entries {
            let thread_id = entry
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            if thread_id.is_empty() {
                continue;
            }

            let unread = entry
                .get("unread")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let reason = entry
                .get("reason")
                .and_then(Value::as_str)
                .map(ToString::to_string);
            let updated_at = entry
                .get("updated_at")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();

            let repository = entry
                .get("repository")
                .and_then(|value| value.get("full_name"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            if repository.is_empty() {
                continue;
            }

            let subject_type = entry
                .get("subject")
                .and_then(|value| value.get("type"))
                .and_then(Value::as_str)
                .map(ToString::to_string);
            let subject_title = entry
                .get("subject")
                .and_then(|value| value.get("title"))
                .and_then(Value::as_str)
                .unwrap_or("(untitled)")
                .to_string();
            let raw_subject_url = entry
                .get("subject")
                .and_then(|value| value.get("url"))
                .and_then(Value::as_str)
                .map(ToString::to_string);

            let pr_url = raw_subject_url
                .as_deref()
                .and_then(|url| api_url_to_pr_url(url, subject_type.as_deref()));
            let issue_api_url = match subject_type.as_deref() {
                Some("Issue") => raw_subject_url.clone(),
                _ => None,
            };
            let subject_url = raw_subject_url
                .as_deref()
                .and_then(api_url_to_html_url)
                .or(raw_subject_url.clone());

            results.push(NotificationThread {
                thread_id,
                unread,
                reason,
                updated_at,
                repository,
                subject_type,
                subject_title,
                subject_url,
                pr_url,
                issue_api_url,
                issue_state: None,
            });
        }
    }

    Ok(results)
}

fn notifications_endpoint(since: Option<&str>) -> String {
    match since {
        Some(since) => format!("/notifications?since={}", encode_query_component(since)),
        None => "/notifications".to_string(),
    }
}

fn encode_query_component(value: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        let is_unreserved = matches!(
            byte,
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~'
        );
        if is_unreserved {
            encoded.push(char::from(byte));
        } else {
            encoded.push('%');
            encoded.push(char::from(HEX[usize::from(byte >> 4)]));
            encoded.push(char::from(HEX[usize::from(byte & 0x0F)]));
        }
    }
    encoded
}

pub async fn fetch_authored_prs(since: Option<&str>) -> anyhow::Result<Vec<AuthoredPrSummary>> {
    let mut args = vec![
        "search",
        "prs",
        "--author",
        "@me",
        "--limit",
        "200",
        "--json",
        "url,title,updatedAt,repository,state,isDraft",
    ];
    let updated_filter;
    if let Some(since) = since {
        updated_filter = format!(">={since}");
        args.push("--updated");
        args.push(&updated_filter);
    }
    let output = Cmd::new("gh", args).run().await?;

    output.ensure_success("❌ Failed to fetch authored pull requests")?;
    if output.stdout().trim().is_empty() {
        return Ok(Vec::new());
    }

    let value: Value = serde_json::from_str(output.stdout()).context("Invalid authored PR JSON")?;
    let mut results = Vec::new();

    let Value::Array(items) = value else {
        return Ok(results);
    };

    for item in items {
        let pr_url = item
            .get("url")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        if pr_url.is_empty() {
            continue;
        }

        let title = item
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("(untitled)")
            .to_string();
        let updated_at = item
            .get("updatedAt")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let is_open = item
            .get("state")
            .and_then(Value::as_str)
            .map(|state| state.eq_ignore_ascii_case("open"))
            .unwrap_or(true);
        let is_draft = item
            .get("isDraft")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let repository = item
            .get("repository")
            .and_then(|value| value.get("nameWithOwner"))
            .and_then(Value::as_str)
            .map(ToString::to_string)
            .or_else(|| {
                item.get("repository")
                    .and_then(|value| value.get("fullName"))
                    .and_then(Value::as_str)
                    .map(ToString::to_string)
            })
            .unwrap_or_else(|| parse_repo_from_pr_url(&pr_url).unwrap_or_default());

        if repository.is_empty() {
            continue;
        }

        results.push(AuthoredPrSummary {
            pr_url,
            repository,
            title,
            updated_at,
            is_open,
            is_draft,
        });
    }

    Ok(results)
}

pub async fn fetch_pr_details(pr_url: &str) -> anyhow::Result<PrDetails> {
    let output = Cmd::new(
        "gh",
        [
            "pr",
            "view",
            pr_url,
            "--json",
            "title,url,state,isDraft,headRefName,headRefOid,baseRefName,createdAt,updatedAt,number,author,headRepository,headRepositoryOwner,isCrossRepository",
        ],
    )
    .run()
    .await?;

    output.ensure_success(format!("❌ Failed to fetch PR details for {pr_url}"))?;
    anyhow::ensure!(
        !output.stdout().trim().is_empty(),
        "❌ Failed to fetch PR details for {pr_url}: empty output"
    );

    let value: Value = serde_json::from_str(output.stdout())?;
    let canonical_pr_url = value
        .get("url")
        .and_then(Value::as_str)
        .unwrap_or(pr_url)
        .to_string();

    let parsed = parse_github_pr_url(&canonical_pr_url)?;
    let is_archived = fetch_repository_archived(&parsed.owner, &parsed.repo).await?;
    let number = i64::try_from(parsed.number)
        .with_context(|| format!("PR number is too large for i64: {}", parsed.number))?;
    let state = value
        .get("state")
        .and_then(Value::as_str)
        .unwrap_or("OPEN")
        .to_string();

    let title = value
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or("(untitled)")
        .to_string();
    let head_ref = value
        .get("headRefName")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let base_ref = value
        .get("baseRefName")
        .and_then(Value::as_str)
        .unwrap_or("main")
        .to_string();
    let head_sha = value
        .get("headRefOid")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let created_at = value
        .get("createdAt")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let updated_at = value
        .get("updatedAt")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let author_login = value
        .get("author")
        .and_then(|value| value.get("login"))
        .and_then(Value::as_str)
        .map(ToString::to_string);
    let head_repo_owner = value
        .get("headRepositoryOwner")
        .and_then(|value| value.get("login"))
        .and_then(Value::as_str)
        .map(ToString::to_string);
    let head_repo_name = value
        .get("headRepository")
        .and_then(|value| value.get("name"))
        .and_then(Value::as_str)
        .map(ToString::to_string);
    let is_cross_repository = value
        .get("isCrossRepository")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let is_draft = value
        .get("isDraft")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    Ok(PrDetails {
        pr_url: canonical_pr_url,
        owner: parsed.owner,
        repo: parsed.repo,
        number,
        state,
        title,
        head_ref,
        base_ref,
        head_sha,
        created_at,
        updated_at,
        is_archived,
        author_login,
        head_repo_owner,
        head_repo_name,
        is_cross_repository,
        is_draft,
    })
}

/// Parse an issue API URL like `https://api.github.com/repos/<org>/<repo>/issues/123`
/// into `(owner, repo, number)`.
struct IssueRef {
    owner: String,
    repo: String,
    number: u64,
}

fn parse_issue_api_url(api_url: &str) -> Option<IssueRef> {
    let path = api_url
        .strip_prefix("https://api.github.com/repos/")
        .or_else(|| api_url.strip_prefix("repos/"))?;
    let parts: Vec<&str> = path.split('/').collect();
    if parts.len() >= 4 && parts[2] == "issues" {
        let number: u64 = parts[3].parse().ok()?;
        return Some(IssueRef {
            owner: parts[0].to_string(),
            repo: parts[1].to_string(),
            number,
        });
    }
    None
}

fn parse_pr_graphql_value(
    pr_val: &Value,
    owner: &str,
    repo: &str,
    is_archived: bool,
) -> anyhow::Result<PrDetails> {
    let number = pr_val
        .get("number")
        .and_then(Value::as_i64)
        .unwrap_or_default();
    let pr_url = format!("https://github.com/{owner}/{repo}/pull/{number}");
    Ok(PrDetails {
        pr_url,
        owner: owner.to_string(),
        repo: repo.to_string(),
        number,
        state: pr_val
            .get("state")
            .and_then(Value::as_str)
            .unwrap_or("OPEN")
            .to_string(),
        title: pr_val
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("(untitled)")
            .to_string(),
        head_ref: pr_val
            .get("headRefName")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        base_ref: pr_val
            .get("baseRefName")
            .and_then(Value::as_str)
            .unwrap_or("main")
            .to_string(),
        head_sha: pr_val
            .get("headRefOid")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        created_at: pr_val
            .get("createdAt")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        updated_at: pr_val
            .get("updatedAt")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        is_archived,
        author_login: pr_val
            .get("author")
            .and_then(|value| value.get("login"))
            .and_then(Value::as_str)
            .map(ToString::to_string),
        head_repo_owner: pr_val
            .get("headRepositoryOwner")
            .and_then(|value| value.get("login"))
            .and_then(Value::as_str)
            .map(ToString::to_string),
        head_repo_name: pr_val
            .get("headRepository")
            .and_then(|value| value.get("name"))
            .and_then(Value::as_str)
            .map(ToString::to_string),
        is_cross_repository: pr_val
            .get("isCrossRepository")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        is_draft: pr_val
            .get("isDraft")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    })
}

/// Maximum number of top-level GraphQL fields per request to stay within
/// GitHub's query complexity limits.
const GRAPHQL_BATCH_CHUNK_SIZE: usize = 25;

/// Fetch details for multiple PRs and issue states using batched GraphQL calls.
/// `pr_urls` are HTML PR URLs like `https://github.com/owner/repo/pull/123`.
/// `issue_api_urls` are REST API URLs like `https://api.github.com/repos/o/r/issues/42`.
pub async fn fetch_batch(
    pr_urls: &[String],
    issue_api_urls: &[String],
) -> anyhow::Result<BatchFetchResult> {
    if pr_urls.is_empty() && issue_api_urls.is_empty() {
        return Ok(BatchFetchResult::default());
    }

    // Deduplicate PRs by (owner, repo, number)
    let mut pr_refs: Vec<(String, String, u64, String)> = Vec::new();
    let mut seen_prs = std::collections::HashSet::new();
    for url in pr_urls {
        if let Ok(parsed) = parse_github_pr_url(url) {
            let key = format!("{}/{}/{}", parsed.owner, parsed.repo, parsed.number);
            if seen_prs.insert(key) {
                pr_refs.push((parsed.owner, parsed.repo, parsed.number, url.clone()));
            }
        }
    }

    // Deduplicate issues
    let mut issue_refs: Vec<(IssueRef, String)> = Vec::new();
    let mut seen_issues = std::collections::HashSet::new();
    for url in issue_api_urls {
        if let Some(issue) = parse_issue_api_url(url) {
            let key = format!("{}/{}/{}", issue.owner, issue.repo, issue.number);
            if seen_issues.insert(key) {
                issue_refs.push((issue, url.clone()));
            }
        }
    }

    let mut result = BatchFetchResult::default();

    // Process in chunks to avoid GitHub GraphQL resource limits.
    let pr_chunks: Vec<&[(String, String, u64, String)]> =
        pr_refs.chunks(GRAPHQL_BATCH_CHUNK_SIZE).collect();
    let issue_chunks: Vec<&[(IssueRef, String)]> =
        issue_refs.chunks(GRAPHQL_BATCH_CHUNK_SIZE).collect();
    let total_chunks = pr_chunks.len().max(issue_chunks.len());

    for chunk_idx in 0..total_chunks {
        let pr_chunk = pr_chunks.get(chunk_idx).copied().unwrap_or(&[]);
        let issue_chunk = issue_chunks.get(chunk_idx).copied().unwrap_or(&[]);

        if pr_chunk.is_empty() && issue_chunk.is_empty() {
            continue;
        }

        let chunk_result = fetch_batch_chunk(pr_chunk, issue_chunk).await?;

        result.pr_details.extend(chunk_result.pr_details);
        result.issue_states.extend(chunk_result.issue_states);
        result.participants.extend(chunk_result.participants);
    }

    Ok(result)
}

/// Execute a single GraphQL batch request for a chunk of PRs and issues.
async fn fetch_batch_chunk(
    pr_chunk: &[(String, String, u64, String)],
    issue_chunk: &[(IssueRef, String)],
) -> anyhow::Result<BatchFetchResult> {
    let pr_fields = "number title state isDraft headRefName headRefOid baseRefName \
                     createdAt updatedAt author { login avatarUrl } \
                     headRepository { name } headRepositoryOwner { login } \
                     isCrossRepository \
                     participants(first: 10) { nodes { login avatarUrl } } \
                     timelineItems(last: 30, itemTypes: [ISSUE_COMMENT, PULL_REQUEST_REVIEW, PULL_REQUEST_COMMIT, PULL_REQUEST_REVIEW_THREAD]) { \
                       nodes { \
                         __typename \
                         ... on IssueComment { createdAt author { login avatarUrl } } \
                         ... on PullRequestReview { createdAt author { login avatarUrl } } \
                         ... on PullRequestCommit { commit { authoredDate author { user { login avatarUrl } } } } \
                       } \
                     }";

    let mut query = String::from("query {");

    for (index, (owner, repo, number, _)) in pr_chunk.iter().enumerate() {
        write!(
            query,
            " pr{index}: repository(owner: \"{owner}\", name: \"{repo}\") {{ \
                isArchived pullRequest(number: {number}) {{ {pr_fields} }} \
            }}"
        )?;
    }

    let issue_fields = "state \
                       author { login avatarUrl } \
                       participants(first: 10) { nodes { login avatarUrl } } \
                       timelineItems(last: 30, itemTypes: [ISSUE_COMMENT]) { \
                         nodes { \
                           __typename \
                           ... on IssueComment { createdAt author { login avatarUrl } } \
                         } \
                       }";

    for (index, (issue, _)) in issue_chunk.iter().enumerate() {
        write!(
            query,
            " issue{index}: repository(owner: \"{}\", name: \"{}\") {{ \
                issue(number: {}) {{ {issue_fields} }} \
            }}",
            issue.owner, issue.repo, issue.number
        )?;
    }

    query.push('}');

    let output = Cmd::new("gh", ["api", "graphql", "-f", &format!("query={query}")])
        .run()
        .await?;
    output.ensure_success("❌ Failed to run batch GraphQL query")?;

    let response: Value =
        serde_json::from_str(output.stdout()).context("Invalid batch GraphQL JSON")?;
    let data = response.get("data").unwrap_or(&Value::Null);

    let mut result = BatchFetchResult::default();

    for (index, (owner, repo, _, pr_url)) in pr_chunk.iter().enumerate() {
        let alias = format!("pr{index}");
        let Some(repo_val) = data.get(&alias) else {
            eprintln!("⚠️ Missing GraphQL alias {alias} for {pr_url}");
            continue;
        };
        if repo_val.is_null() {
            eprintln!("⚠️ GraphQL returned null for {alias} ({pr_url})");
            continue;
        }
        let is_archived = repo_val
            .get("isArchived")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let Some(pr_val) = repo_val.get("pullRequest") else {
            eprintln!("⚠️ No pullRequest in {alias} for {pr_url}");
            continue;
        };
        if pr_val.is_null() {
            eprintln!("⚠️ pullRequest is null for {pr_url}");
            continue;
        }
        match parse_pr_graphql_value(pr_val, owner, repo, is_archived) {
            Ok(details) => {
                result.pr_details.insert(pr_url.clone(), details);
            }
            Err(err) => {
                eprintln!("⚠️ Failed to parse PR details for {pr_url}: {err}");
            }
        }

        let mut participants = pr_val
            .get("participants")
            .and_then(|value| value.get("nodes"))
            .and_then(Value::as_array)
            .map(|nodes| {
                nodes
                    .iter()
                    .filter_map(|node| {
                        let login = node.get("login")?.as_str()?.to_string();
                        let avatar_url = node.get("avatarUrl")?.as_str()?.to_string();
                        Some(Participant {
                            login,
                            avatar_url,
                            last_activity_at: None,
                        })
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        // Extract per-user last activity timestamps (and avatar URLs) from timelineItems.
        let activity_map = extract_participant_activity(pr_val);
        for participant in &mut participants {
            if let Some((ts, _)) = activity_map.get(&participant.login) {
                participant.last_activity_at = Some(ts.clone());
            }
        }

        // Include users from timeline items who aren't in the participants list
        // (e.g. bots like github-actions[bot] which GitHub's participants field omits).
        for (login, (ts, avatar_url)) in &activity_map {
            if !participants
                .iter()
                .any(|participant| participant.login == *login)
                && let Some(avatar_url) = avatar_url
            {
                participants.push(Participant {
                    login: login.clone(),
                    avatar_url: avatar_url.clone(),
                    last_activity_at: Some(ts.clone()),
                });
            }
        }

        // Include the PR author if not already in the participants list.
        if let Some(author) = pr_val.get("author")
            && let (Some(login), Some(avatar_url)) = (
                author.get("login").and_then(Value::as_str),
                author.get("avatarUrl").and_then(Value::as_str),
            )
            && !participants
                .iter()
                .any(|participant| participant.login == login)
        {
            let last_activity_at = activity_map.get(login).map(|(ts, _)| ts.clone());
            participants.insert(
                0,
                Participant {
                    login: login.to_string(),
                    avatar_url: avatar_url.to_string(),
                    last_activity_at,
                },
            );
        }

        // Sort participants by last activity (most recent first).
        // Participants without activity data sink to the end.
        participants.sort_by(|left, right| {
            right
                .last_activity_at
                .as_deref()
                .cmp(&left.last_activity_at.as_deref())
        });

        if !participants.is_empty() {
            result.participants.insert(pr_url.clone(), participants);
        }
    }

    for (index, (issue, api_url)) in issue_chunk.iter().enumerate() {
        let alias = format!("issue{index}");
        let Some(repo_val) = data.get(&alias) else {
            continue;
        };
        let Some(issue_val) = repo_val.get("issue") else {
            continue;
        };
        if issue_val.is_null() {
            continue;
        }

        if let Some(state) = issue_val.get("state").and_then(Value::as_str) {
            result
                .issue_states
                .insert(api_url.clone(), state.to_ascii_uppercase());
        }

        // Extract participants for the issue, mirroring the PR participant logic.
        let html_url = format!(
            "https://github.com/{}/{}/issues/{}",
            issue.owner, issue.repo, issue.number
        );

        let mut participants = issue_val
            .get("participants")
            .and_then(|value| value.get("nodes"))
            .and_then(Value::as_array)
            .map(|nodes| {
                nodes
                    .iter()
                    .filter_map(|node| {
                        let login = node.get("login")?.as_str()?.to_string();
                        let avatar_url = node.get("avatarUrl")?.as_str()?.to_string();
                        Some(Participant {
                            login,
                            avatar_url,
                            last_activity_at: None,
                        })
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        let activity_map = extract_participant_activity(issue_val);
        for participant in &mut participants {
            if let Some((ts, _)) = activity_map.get(&participant.login) {
                participant.last_activity_at = Some(ts.clone());
            }
        }

        for (login, (ts, avatar_url)) in &activity_map {
            if !participants
                .iter()
                .any(|participant| participant.login == *login)
                && let Some(avatar_url) = avatar_url
            {
                participants.push(Participant {
                    login: login.clone(),
                    avatar_url: avatar_url.clone(),
                    last_activity_at: Some(ts.clone()),
                });
            }
        }

        // Include the issue author if not already present.
        if let Some(author) = issue_val.get("author")
            && let (Some(login), Some(avatar_url)) = (
                author.get("login").and_then(Value::as_str),
                author.get("avatarUrl").and_then(Value::as_str),
            )
            && !participants
                .iter()
                .any(|participant| participant.login == login)
        {
            let last_activity_at = activity_map.get(login).map(|(ts, _)| ts.clone());
            participants.insert(
                0,
                Participant {
                    login: login.to_string(),
                    avatar_url: avatar_url.to_string(),
                    last_activity_at,
                },
            );
        }

        participants.sort_by(|left, right| {
            right
                .last_activity_at
                .as_deref()
                .cmp(&left.last_activity_at.as_deref())
        });

        if !participants.is_empty() {
            result.participants.insert(html_url, participants);
        }
    }

    Ok(result)
}

/// Extracts a map of `login -> latest_timestamp` from the PR's `timelineItems`.
///
/// Walks through comments, reviews, and commits to find the most recent
/// activity timestamp for each user.
fn extract_participant_activity(pr_val: &Value) -> HashMap<String, (String, Option<String>)> {
    let mut activity: HashMap<String, (String, Option<String>)> = HashMap::new();

    let Some(nodes) = pr_val
        .get("timelineItems")
        .and_then(|value| value.get("nodes"))
        .and_then(Value::as_array)
    else {
        return activity;
    };

    for node in nodes {
        let typename = node.get("__typename").and_then(Value::as_str).unwrap_or("");
        let (login, timestamp, avatar_url) = match typename {
            "IssueComment" | "PullRequestReview" => {
                let author = node.get("author");
                let login = author
                    .and_then(|author| author.get("login"))
                    .and_then(Value::as_str);
                let ts = node.get("createdAt").and_then(Value::as_str);
                let avatar = author
                    .and_then(|author| author.get("avatarUrl"))
                    .and_then(Value::as_str);
                (login, ts, avatar)
            }
            "PullRequestCommit" => {
                let commit = node.get("commit");
                let author = commit.and_then(|commit| commit.get("author"));
                let user = author.and_then(|author| author.get("user"));
                let login = user
                    .and_then(|user| user.get("login"))
                    .and_then(Value::as_str);
                let ts = commit
                    .and_then(|commit| commit.get("authoredDate"))
                    .and_then(Value::as_str);
                let avatar = user
                    .and_then(|user| user.get("avatarUrl"))
                    .and_then(Value::as_str);
                (login, ts, avatar)
            }
            _ => (None, None, None),
        };

        if let (Some(login), Some(ts)) = (login, timestamp) {
            let entry = activity
                .entry(login.to_string())
                .or_insert_with(|| (String::new(), None));
            if entry.0.is_empty() || ts > entry.0.as_str() {
                entry.0 = ts.to_string();
            }
            if entry.1.is_none() {
                entry.1 = avatar_url.map(|avatar| avatar.to_string());
            }
        }
    }

    activity
}

async fn fetch_repository_archived(owner: &str, repo: &str) -> anyhow::Result<bool> {
    let endpoint = format!("/repos/{owner}/{repo}");
    let output = Cmd::new("gh", ["api", &endpoint]).run().await?;
    output.ensure_success(format!(
        "❌ Failed to fetch repository details for {owner}/{repo}"
    ))?;
    anyhow::ensure!(
        !output.stdout().trim().is_empty(),
        "❌ Failed to fetch repository details for {owner}/{repo}: empty output"
    );

    let value: Value =
        serde_json::from_str(output.stdout()).context("Invalid repository details JSON")?;
    Ok(value
        .get("archived")
        .and_then(Value::as_bool)
        .unwrap_or(false))
}

pub async fn mark_notification_done(thread_id: &str) -> anyhow::Result<()> {
    let endpoint = format!("/notifications/threads/{thread_id}");
    let output = Cmd::new("gh", ["api", "-X", "DELETE", &endpoint])
        .run()
        .await?;
    output.ensure_success("❌ Failed to mark notification thread as done")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notifications_endpoint_encodes_since_cursor() {
        let endpoint = notifications_endpoint(Some("2026-03-13T09:00:00+00:00"));
        assert_eq!(
            endpoint,
            "/notifications?since=2026-03-13T09%3A00%3A00%2B00%3A00"
        );
    }
}
