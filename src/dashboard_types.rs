use serde::{Deserialize, Serialize};

/// Thread displayed on the dashboard. Shared between server and client.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DashboardThread {
    pub thread_key: String,
    pub github_thread_id: Option<String>,
    pub sources: Vec<String>,
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
    pub latest_requires_code_changes: Option<bool>,
    pub pr_state: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct DashboardFilters {
    pub show_notifications: bool,
    pub show_prs: bool,
    pub show_done: bool,
    pub show_not_done: bool,
    pub group_by_repository: bool,
}

impl Default for DashboardFilters {
    fn default() -> Self {
        Self {
            show_notifications: true,
            show_prs: true,
            show_done: false,
            show_not_done: true,
            group_by_repository: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PollStats {
    pub notifications_fetched: usize,
    pub authored_prs_fetched: usize,
    pub prs_seen: usize,
    pub reviews_run: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredReview {
    pub id: i64,
    pub pr_url: String,
    pub provider: String,
    pub model: Option<String>,
    pub requires_code_changes: bool,
    pub content_md: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarkDonePayload {
    pub github_thread_id: Option<String>,
    pub pr_url: Option<String>,
    pub mark_authored_pr: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenProjectRequest {
    pub repository: String,
    pub pr_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FixResponse {
    pub output: String,
}

/// Events pushed to the dashboard via SSE.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum DashboardEvent {
    #[serde(rename = "poll_complete")]
    PollComplete(PollStats),
    #[serde(rename = "review_complete")]
    ReviewComplete { pr_url: String },
}
