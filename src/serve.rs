use std::{collections::HashSet, sync::Arc, time::Duration};

use anyhow::Context as _;
use camino::{Utf8Path, Utf8PathBuf};
use serde::Serialize;

use crate::{
    config::{self, AiProvider, AppConfig, RereviewMode},
    db::{self, Db},
    github, review, web,
};

#[derive(Debug, Clone)]
pub struct AppState {
    pub db: Db,
    pub config: AppConfig,
    pub work_dir: Utf8PathBuf,
    pub poll_lock: Arc<tokio::sync::Mutex<()>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PollStats {
    pub notifications_fetched: usize,
    pub authored_prs_fetched: usize,
    pub prs_seen: usize,
    pub reviews_run: usize,
}

pub fn run_serve() -> anyhow::Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("Failed to initialize tokio runtime")?;

    let result = runtime.block_on(run_serve_async());
    runtime.shutdown_timeout(Duration::from_secs(1));
    result
}

async fn run_serve_async() -> anyhow::Result<()> {
    let paths = config::resolve_paths()?;
    config::ensure_parent_dirs(&paths)?;

    let cfg = config::load_config(&paths.config_path)?;
    let db = Db::new(&paths.db_path)?;
    web::prepare_dashboard_assets(&paths.dashboard_dir)?;

    let current_dir = std::env::current_dir().context("Failed to read current directory")?;
    let work_dir = Utf8PathBuf::from_path_buf(current_dir)
        .map_err(|p| anyhow::anyhow!("Current directory is not valid UTF-8: {}", p.display()))?;

    let state = Arc::new(AppState {
        db,
        config: cfg.clone(),
        work_dir,
        poll_lock: Arc::new(tokio::sync::Mutex::new(())),
    });

    let browser_url = dashboard_browser_url(&cfg);
    println!(
        "🚀 gigi serve: bind {}:{}, open {}",
        cfg.dashboard.host, cfg.dashboard.port, browser_url
    );
    println!("📄 Config: {}", paths.config_path.display());
    println!("🗄️  DB: {}", paths.db_path.display());

    let poll_state = Arc::clone(&state);
    let poll_handle = tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(
            poll_state.config.watch_period_seconds.max(1),
        ));

        loop {
            interval.tick().await;
            if let Err(err) = poll_state.poll_once().await {
                eprintln!("⚠️ Poll cycle failed: {err}");
            }
        }
    });

    tokio::select! {
        server_result = web::run_server(state, &cfg, &paths.dashboard_dir) => {
            poll_handle.abort();
            server_result
        }
        signal_result = tokio::signal::ctrl_c() => {
            poll_handle.abort();
            match signal_result {
                Ok(()) => {
                    println!("\n🛑 Received Ctrl+C, shutting down gigi serve...");
                    Ok(())
                }
                Err(err) => Err(anyhow::anyhow!("Failed to listen for Ctrl+C: {err}")),
            }
        }
    }
}

impl AppState {
    pub async fn poll_once(&self) -> anyhow::Result<PollStats> {
        let _guard = self.poll_lock.lock().await;

        let db = self.db.clone();
        let config = self.config.clone();
        let work_dir = self.work_dir.clone();

        let handle =
            tokio::task::spawn_blocking(move || poll_once_blocking(&db, &config, &work_dir));
        handle
            .await
            .context("polling task join failure")?
            .context("polling cycle failed")
    }

    pub async fn mark_done(&self, thread_id: String) -> anyhow::Result<()> {
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || {
            github::mark_notification_done(&thread_id)?;
            db.mark_thread_done_local(&thread_id)?;
            Ok(())
        })
        .await
        .context("mark-done task join failure")?
    }

    pub async fn run_fix(
        &self,
        owner: String,
        repo: String,
        number: i64,
    ) -> anyhow::Result<String> {
        let db = self.db.clone();
        let provider = self.config.ai.provider;
        let model = self.config.ai.model.clone();

        tokio::task::spawn_blocking(move || {
            let pr_url = format!("https://github.com/{owner}/{repo}/pull/{number}");
            let latest_review = db
                .latest_review_by_url(&pr_url)?
                .ok_or_else(|| anyhow::anyhow!("No review found for {pr_url}"))?;

            let repo_dir = github::ensure_local_repo(&owner, &repo)?;
            github::checkout_pr(&repo_dir, &pr_url)?;

            let agent = provider.as_agent();
            let output = review::run_fix(
                &repo_dir,
                &pr_url,
                &latest_review.content_md,
                Some(&agent),
                model.as_deref(),
            );

            match output {
                Ok(text) => {
                    db.insert_fix_run(&pr_url, provider_name(provider), "success", &text)?;
                    Ok(text)
                }
                Err(err) => {
                    db.insert_fix_run(&pr_url, provider_name(provider), "error", &err.to_string())?;
                    Err(err)
                }
            }
        })
        .await
        .context("fix task join failure")?
    }
}

fn poll_once_blocking(
    db: &Db,
    config: &AppConfig,
    work_dir: &Utf8Path,
) -> anyhow::Result<PollStats> {
    let notifications = github::fetch_notifications()?;
    let authored_prs = github::fetch_authored_open_prs()?;

    let mut pr_urls = HashSet::new();

    for notification in &notifications {
        let thread_key = format!("notif:{}", notification.thread_id);
        db.upsert_thread(&db::NewThread {
            thread_key,
            github_thread_id: Some(notification.thread_id.clone()),
            source: "notification".to_string(),
            repository: notification.repository.clone(),
            subject_type: notification.subject_type.clone(),
            subject_title: notification.subject_title.clone(),
            subject_url: notification.subject_url.clone(),
            reason: notification.reason.clone(),
            pr_url: notification.pr_url.clone(),
            unread: notification.unread,
            done: false,
            updated_at: notification.updated_at.clone(),
        })?;

        if let Some(pr_url) = &notification.pr_url {
            pr_urls.insert(pr_url.clone());
        }
    }

    for authored in &authored_prs {
        let thread_key = format!("mypr:{}", authored.pr_url);
        db.upsert_thread(&db::NewThread {
            thread_key,
            github_thread_id: None,
            source: "my_pr".to_string(),
            repository: authored.repository.clone(),
            subject_type: Some("PullRequest".to_string()),
            subject_title: authored.title.clone(),
            subject_url: Some(authored.pr_url.clone()),
            reason: Some("authored".to_string()),
            pr_url: Some(authored.pr_url.clone()),
            unread: false,
            done: false,
            updated_at: authored.updated_at.clone(),
        })?;

        pr_urls.insert(authored.pr_url.clone());
    }

    let mut reviews_run = 0_usize;

    for pr_url in &pr_urls {
        let details = match github::fetch_pr_details(pr_url) {
            Ok(details) => details,
            Err(err) => {
                eprintln!("⚠️ Failed to fetch PR details for {pr_url}: {err}");
                continue;
            }
        };

        db.upsert_pr(&db::NewPr {
            pr_url: details.pr_url.clone(),
            owner: details.owner.clone(),
            repo: details.repo.clone(),
            number: details.number,
            state: details.state.clone(),
            title: details.title.clone(),
            head_ref: details.head_ref.clone(),
            base_ref: details.base_ref.clone(),
            head_sha: details.head_sha.clone(),
            updated_at: details.updated_at.clone(),
        })?;

        let stored = db.get_pr(&details.pr_url)?;
        let should_review = should_review_pr(config.rereview_mode, stored.as_ref(), &details);
        if should_review {
            let agent = config.ai.provider.as_agent();
            match review::generate_review(
                work_dir,
                &details.pr_url,
                Some(&agent),
                config.ai.model.as_deref(),
            ) {
                Ok(review_result) => {
                    db.insert_review(&db::NewReview {
                        pr_url: details.pr_url.clone(),
                        provider: review_result.provider,
                        model: review_result.model,
                        requires_code_changes: review_result.requires_code_changes,
                        content_md: review_result.markdown,
                    })?;
                    db.set_pr_review_marker(
                        &details.pr_url,
                        &details.head_sha,
                        &details.updated_at,
                    )?;
                    reviews_run += 1;
                }
                Err(err) => {
                    eprintln!("⚠️ Review failed for {}: {err}", details.pr_url);
                }
            }
        }

        if details.state != "OPEN"
            && let Err(err) = handle_closed_pr_branch_sync(db, &details)
        {
            eprintln!(
                "⚠️ Failed to process closed PR branch sync for {}: {err}",
                details.pr_url
            );
            drop(db.insert_sync_event(&details.pr_url, "error", &err.to_string()));
        }
    }

    Ok(PollStats {
        notifications_fetched: notifications.len(),
        authored_prs_fetched: authored_prs.len(),
        prs_seen: pr_urls.len(),
        reviews_run,
    })
}

fn should_review_pr(
    mode: RereviewMode,
    stored: Option<&db::StoredPr>,
    details: &github::PrDetails,
) -> bool {
    let Some(stored) = stored else {
        return true;
    };

    if stored.last_reviewed_sha.is_none() || stored.last_reviewed_updated_at.is_none() {
        return true;
    }

    match mode {
        RereviewMode::Manual => false,
        RereviewMode::OnUpdate => {
            stored.last_reviewed_sha.as_deref() != Some(details.head_sha.as_str())
                || stored.last_reviewed_updated_at.as_deref() != Some(details.updated_at.as_str())
        }
    }
}

fn handle_closed_pr_branch_sync(db: &Db, details: &github::PrDetails) -> anyhow::Result<()> {
    let repo_dir = github::local_repo_dir(&details.owner, &details.repo)?;
    if !repo_dir.exists() || !repo_dir.join(".git").exists() {
        return Ok(());
    }

    let current_branch = github::current_branch(&repo_dir)?;
    if current_branch != details.head_ref {
        return Ok(());
    }

    if !github::is_clean_repo(&repo_dir)? {
        let message = format!(
            "Skipped sync for {} because working tree is dirty on branch '{}'.",
            details.pr_url, details.head_ref
        );
        db.insert_sync_event(&details.pr_url, "warning", &message)?;
        return Ok(());
    }

    let default_branch = github::default_branch(&repo_dir)?;
    github::checkout_branch(&repo_dir, &default_branch)?;
    github::pull_ff_only(&repo_dir)?;

    let message = format!(
        "Switched to default branch '{default_branch}' and pulled latest changes after PR closed."
    );
    db.insert_sync_event(&details.pr_url, "success", &message)?;

    Ok(())
}

fn provider_name(provider: AiProvider) -> &'static str {
    match provider {
        AiProvider::Copilot => "copilot",
        AiProvider::Gemini => "gemini",
        AiProvider::Kiro => "kiro",
    }
}

fn dashboard_browser_url(config: &AppConfig) -> String {
    let host = match config.dashboard.host.as_str() {
        "0.0.0.0" | "::" => "localhost",
        other => other,
    };
    format!("http://{host}:{}", config.dashboard.port)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rereview_on_update() {
        let details = github::PrDetails {
            pr_url: "u".to_string(),
            owner: "o".to_string(),
            repo: "r".to_string(),
            number: 1,
            state: "OPEN".to_string(),
            title: "t".to_string(),
            head_ref: "feat".to_string(),
            base_ref: "main".to_string(),
            head_sha: "sha2".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
        };
        let stored = db::StoredPr {
            pr_url: "u".to_string(),
            owner: "o".to_string(),
            repo: "r".to_string(),
            number: 1,
            state: "OPEN".to_string(),
            title: "t".to_string(),
            head_ref: "feat".to_string(),
            base_ref: "main".to_string(),
            head_sha: "sha1".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            last_reviewed_sha: Some("sha1".to_string()),
            last_reviewed_updated_at: Some("2026-01-01T00:00:00Z".to_string()),
        };

        assert!(should_review_pr(
            RereviewMode::OnUpdate,
            Some(&stored),
            &details
        ));
    }

    #[test]
    fn manual_mode_skips_after_first_review() {
        let details = github::PrDetails {
            pr_url: "u".to_string(),
            owner: "o".to_string(),
            repo: "r".to_string(),
            number: 1,
            state: "OPEN".to_string(),
            title: "t".to_string(),
            head_ref: "feat".to_string(),
            base_ref: "main".to_string(),
            head_sha: "sha1".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
        };
        let stored = db::StoredPr {
            pr_url: "u".to_string(),
            owner: "o".to_string(),
            repo: "r".to_string(),
            number: 1,
            state: "OPEN".to_string(),
            title: "t".to_string(),
            head_ref: "feat".to_string(),
            base_ref: "main".to_string(),
            head_sha: "sha1".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            last_reviewed_sha: Some("sha1".to_string()),
            last_reviewed_updated_at: Some("2026-01-01T00:00:00Z".to_string()),
        };

        assert!(!should_review_pr(
            RereviewMode::Manual,
            Some(&stored),
            &details
        ));
    }

    #[test]
    fn wildcard_host_uses_localhost_in_browser_url() {
        let cfg = AppConfig {
            dashboard: crate::config::DashboardConfig {
                host: "0.0.0.0".to_string(),
                port: 8787,
            },
            ..AppConfig::default()
        };

        assert_eq!(dashboard_browser_url(&cfg), "http://localhost:8787");
    }
}
