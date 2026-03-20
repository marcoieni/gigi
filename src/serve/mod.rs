mod app;
mod helpers;
mod poll;
mod time;

#[cfg(test)]
mod tests;

use std::{
    collections::HashMap,
    sync::{Arc, atomic::AtomicBool},
};

use camino::Utf8PathBuf;
use serde::Serialize;

use crate::{config::AppConfig, db::Db, github};

pub use app::run_serve;

#[derive(Debug)]
pub struct AppState {
    pub db: Db,
    pub config: AppConfig,
    pub work_dir: Utf8PathBuf,
    pub poll_lock: Arc<tokio::sync::Mutex<()>>,
    pub dashboard_refresh_in_flight: Arc<AtomicBool>,
    pub dashboard_updates: tokio::sync::watch::Sender<DashboardUpdate>,
}

#[derive(Debug, Clone)]
pub struct DashboardUpdate {
    pub version: u64,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PollStats {
    pub notifications_fetched: usize,
    pub authored_prs_fetched: usize,
    pub assigned_issues_fetched: usize,
    pub prs_seen: usize,
    pub reviews_run: usize,
    #[serde(skip_serializing)]
    pub participants: HashMap<String, Vec<github::Participant>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PollMode {
    Startup,
    Regular,
    DashboardRefresh,
}

#[derive(Debug, Clone, Copy)]
struct StartupReviewLimits {
    lookback_days: u64,
    max_prs: usize,
}

#[derive(Debug, Default)]
struct StartupReviewSelection {
    to_review: Vec<github::PrDetails>,
    to_mark_baseline: Vec<github::PrDetails>,
}

#[derive(Debug, Clone)]
pub struct MarkDoneRequest {
    pub github_thread_id: Option<String>,
    pub pr_url: Option<String>,
    pub subject_url: Option<String>,
    pub mark_authored_pr: bool,
    pub mark_assigned_issue: bool,
}
