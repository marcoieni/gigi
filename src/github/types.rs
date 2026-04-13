use std::collections::HashMap;

use camino::Utf8PathBuf;

#[derive(Debug, Clone)]
pub struct NotificationThread {
    pub thread_id: String,
    pub unread: bool,
    pub reason: Option<String>,
    pub updated_at: String,
    pub repository: String,
    pub subject_type: Option<String>,
    pub subject_title: String,
    pub subject_url: Option<String>,
    pub pr_url: Option<String>,
    pub issue_api_url: Option<String>,
    pub discussion_api_url: Option<String>,
    pub issue_state: Option<String>,
    pub discussion_answered: Option<bool>,
}

#[derive(Debug, Clone)]
pub struct LocalPrRepo {
    pub repo_dir: Utf8PathBuf,
    pub details: PrDetails,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct GitHubRepoRef {
    pub owner: String,
    pub repo: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CloneTarget {
    pub origin: GitHubRepoRef,
    pub upstream: Option<GitHubRepoRef>,
}

/// A GitHub user who participated in a PR (for avatar display).
#[derive(Debug, Clone)]
pub struct Participant {
    pub login: String,
    pub avatar_url: String,
    /// ISO 8601 timestamp of the participant's most recent activity on the PR.
    /// Used to order participants by recency in the dashboard.
    pub last_activity_at: Option<String>,
}

#[derive(Debug, Clone)]
pub struct AuthoredPrSummary {
    pub pr_url: String,
    pub repository: String,
    pub title: String,
    pub updated_at: String,
    pub is_open: bool,
    pub is_draft: bool,
}

#[derive(Debug, Clone)]
pub struct AssignedIssueSummary {
    pub issue_url: String,
    pub repository: String,
    pub title: String,
    pub updated_at: String,
    pub state: String,
}

#[derive(Debug, Clone)]
pub struct AssignedIssuesSearchResult {
    pub issues: Vec<AssignedIssueSummary>,
    pub is_complete: bool,
}

#[derive(Debug, Clone)]
pub struct PrDetails {
    pub pr_url: String,
    pub owner: String,
    pub repo: String,
    pub number: i64,
    pub state: String,
    pub merge_queue_state: Option<String>,
    pub title: String,
    pub head_ref: String,
    pub base_ref: String,
    pub head_sha: String,
    pub created_at: String,
    pub updated_at: String,
    pub is_archived: bool,
    pub author_login: Option<String>,
    pub head_repo_owner: Option<String>,
    pub head_repo_name: Option<String>,
    pub is_cross_repository: bool,
    pub is_draft: bool,
}

/// Result of a batch GraphQL fetch.
#[derive(Debug, Default)]
pub struct BatchFetchResult {
    pub pr_details: HashMap<String, PrDetails>,
    /// Maps issue API URL to uppercase state string.
    pub issue_states: HashMap<String, String>,
    /// Maps discussion API URL to uppercase state string.
    pub discussion_states: HashMap<String, String>,
    /// Maps discussion API URL to whether the discussion has an accepted answer.
    pub discussion_answers: HashMap<String, bool>,
    /// Maps PR URL to the list of participants (for avatar display).
    pub participants: HashMap<String, Vec<Participant>>,
}
