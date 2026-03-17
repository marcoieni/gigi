use crate::github::Participant;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub struct NewThread {
    pub thread_key: String,
    pub github_thread_id: Option<String>,
    pub source: String,
    pub repository: String,
    pub subject_type: Option<String>,
    pub subject_title: String,
    pub subject_url: Option<String>,
    pub issue_state: Option<String>,
    pub reason: Option<String>,
    pub pr_url: Option<String>,
    pub unread: bool,
    pub done: bool,
    pub updated_at: String,
    pub is_draft: bool,
}

/// Data required to insert a new PR row. Does not include DB-managed fields.
#[derive(Debug, Clone)]
pub struct NewPr {
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
    pub updated_at: String,
    pub is_archived: bool,
    pub is_draft: bool,
}

/// PR row as read from the DB. Extends [`NewPr`] with DB-managed fields
/// (`last_reviewed_sha`, `last_reviewed_updated_at`).
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct StoredPr {
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
    pub updated_at: String,
    pub is_archived: bool,
    pub last_reviewed_sha: Option<String>,
    pub last_reviewed_updated_at: Option<String>,
}

/// Review row as read from the DB. Extends [`NewReview`] with DB-managed fields
/// (`id`, `created_at`).
#[derive(Debug, Clone, Serialize)]
pub struct StoredReview {
    pub id: i64,
    pub pr_url: String,
    pub provider: String,
    pub model: Option<String>,
    pub requires_code_changes: bool,
    pub content_md: String,
    pub created_at: i64,
}

/// Data required to insert a new review row. Does not include DB-managed fields.
#[derive(Debug, Clone)]
pub struct NewReview {
    pub pr_url: String,
    pub provider: String,
    pub model: Option<String>,
    pub requires_code_changes: bool,
    pub content_md: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DashboardThread {
    pub thread_key: String,
    pub github_thread_id: Option<String>,
    pub sources: Vec<String>,
    pub repository: String,
    pub pr_owner: Option<String>,
    pub pr_repo: Option<String>,
    pub pr_number: Option<i64>,
    pub subject_type: Option<String>,
    pub subject_title: String,
    pub subject_url: Option<String>,
    pub issue_state: Option<String>,
    pub reason: Option<String>,
    pub pr_url: Option<String>,
    pub unread: bool,
    pub done: bool,
    pub updated_at: String,
    pub latest_requires_code_changes: Option<bool>,
    pub pr_state: Option<String>,
    pub pr_merge_queue_state: Option<String>,
    pub latest_review_content_md: Option<String>,
    pub latest_review_created_at: Option<i64>,
    pub latest_review_provider: Option<String>,
    pub is_draft: bool,
    /// Participants who interacted with this PR (not persisted, populated at runtime).
    pub participants: Vec<Participant>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DashboardThreadFilters {
    pub show_notifications: bool,
    pub show_prs: bool,
    pub show_done: bool,
    pub show_not_done: bool,
    pub group_by_repository: bool,
    pub hidden_repositories: Vec<String>,
}

impl Default for DashboardThreadFilters {
    fn default() -> Self {
        Self {
            show_notifications: true,
            show_prs: true,
            show_done: false,
            show_not_done: true,
            group_by_repository: true,
            hidden_repositories: Vec::new(),
        }
    }
}
