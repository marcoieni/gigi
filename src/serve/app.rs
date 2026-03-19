use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use anyhow::Context as _;
use camino::Utf8PathBuf;

use crate::{config, github, launcher, review, web};

use super::{
    AppState, DashboardUpdate, MarkDoneRequest, PollMode, PollStats,
    helpers::{dashboard_browser_url, describe_open_target, resolve_open_target_repo},
    poll::{poll_once_async, print_poll_stats, run_review_for_details, upsert_pr_from_details},
};

pub async fn run_serve() -> anyhow::Result<()> {
    let paths = config::resolve_paths()?;
    config::ensure_parent_dirs(&paths).await?;

    let cfg = config::load_config(&paths.config_path).await?;
    let db = crate::db::Db::new(&paths.db_path)?;
    let current_dir = std::env::current_dir().context("Failed to read current directory")?;
    let work_dir = Utf8PathBuf::from_path_buf(current_dir).map_err(|path| {
        anyhow::anyhow!("Current directory is not valid UTF-8: {}", path.display())
    })?;

    let (dashboard_updates, _) = tokio::sync::watch::channel(DashboardUpdate {
        version: 0,
        message: "Waiting for the first poll...".to_string(),
    });

    let state = Arc::new(AppState {
        db,
        config: cfg.clone(),
        work_dir,
        poll_lock: Arc::new(tokio::sync::Mutex::new(())),
        dashboard_refresh_in_flight: Arc::new(AtomicBool::new(false)),
        dashboard_updates,
    });

    let browser_url = dashboard_browser_url(&cfg);
    println!(
        "🚀 gigi serve: bind {}:{}, open {}",
        cfg.dashboard.host, cfg.dashboard.port, browser_url
    );
    println!("📄 Config: {}", paths.config_path.display());
    println!("🗄️  DB: {}", paths.db_path.display());

    let startup_state = Arc::clone(&state);
    let startup_handle = tokio::spawn(async move {
        println!("🔄 Starting initial poll cycle...");
        match startup_state.poll_once_startup().await {
            Ok(stats) => print_poll_stats("✅ Initial poll complete:", &stats),
            Err(err) => eprintln!("⚠️ Initial poll cycle failed: {err:?}"),
        }
    });

    let poll_state = Arc::clone(&state);
    let poll_handle = tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(
            poll_state.config.watch_period_seconds.max(1),
        ));
        interval.tick().await;

        loop {
            interval.tick().await;
            if let Err(err) = poll_state.poll_once_regular().await {
                eprintln!("⚠️ Poll cycle failed: {err}");
            }
        }
    });

    tokio::select! {
        server_result = web::run_server(state, &cfg) => {
            poll_handle.abort();
            startup_handle.abort();
            server_result
        }
        signal_result = tokio::signal::ctrl_c() => {
            poll_handle.abort();
            startup_handle.abort();
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
    pub fn dashboard_status_message(&self) -> String {
        self.dashboard_updates.borrow().message.clone()
    }

    pub fn subscribe_dashboard_updates(&self) -> tokio::sync::watch::Receiver<DashboardUpdate> {
        self.dashboard_updates.subscribe()
    }

    pub fn notify_dashboard(&self, message: impl Into<String>) {
        let next = {
            let current = self.dashboard_updates.borrow().clone();
            DashboardUpdate {
                version: current.version.saturating_add(1),
                message: message.into(),
            }
        };
        drop(self.dashboard_updates.send(next));
    }

    pub fn request_dashboard_refresh(self: &Arc<Self>) {
        if self
            .dashboard_refresh_in_flight
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            self.notify_dashboard("Refresh already in progress...");
            return;
        }

        self.notify_dashboard("Refresh requested...");
        eprintln!("🔄 Dashboard refresh requested");

        let state = Arc::clone(self);
        tokio::spawn(async move {
            let result = state.poll_once_from_dashboard().await;
            state
                .dashboard_refresh_in_flight
                .store(false, Ordering::Release);

            if let Err(err) = result {
                eprintln!("⚠️ Dashboard refresh task failed: {err}");
            }
        });
    }

    async fn poll_once_from_dashboard(&self) -> anyhow::Result<PollStats> {
        let result = self.poll_once_with_mode(PollMode::DashboardRefresh).await;
        match &result {
            Ok(stats) => {
                print_poll_stats("✅ Dashboard refresh complete:", stats);
                self.notify_dashboard(format!(
                    "Refresh complete: notifications={}, my_prs={}, prs={}, reviews={}",
                    stats.notifications_fetched,
                    stats.authored_prs_fetched,
                    stats.prs_seen,
                    stats.reviews_run
                ));
            }
            Err(err) => {
                eprintln!("❌ Dashboard refresh failed: {err}");
                self.notify_dashboard(format!("Refresh failed: {err}"));
            }
        }
        result
    }

    pub async fn poll_once_startup(&self) -> anyhow::Result<PollStats> {
        let result = self.poll_once_with_mode(PollMode::Startup).await;
        match &result {
            Ok(stats) => self.notify_dashboard(format!(
                "Initial poll complete: notifications={}, my_prs={}, prs={}, reviews={}",
                stats.notifications_fetched,
                stats.authored_prs_fetched,
                stats.prs_seen,
                stats.reviews_run
            )),
            Err(err) => self.notify_dashboard(format!("Initial poll failed: {err}")),
        }
        result
    }

    pub async fn poll_once_regular(&self) -> anyhow::Result<PollStats> {
        let result = self.poll_once_with_mode(PollMode::Regular).await;
        match &result {
            Ok(stats) => self.notify_dashboard(format!(
                "Background poll complete: notifications={}, my_prs={}, prs={}, reviews={}",
                stats.notifications_fetched,
                stats.authored_prs_fetched,
                stats.prs_seen,
                stats.reviews_run
            )),
            Err(err) => self.notify_dashboard(format!("Background poll failed: {err}")),
        }
        result
    }

    async fn poll_once_with_mode(&self, mode: PollMode) -> anyhow::Result<PollStats> {
        let _guard = self.poll_lock.lock().await;

        let stats = poll_once_async(&self.db, &self.config, &self.work_dir, mode)
            .await
            .context("polling cycle failed")?;

        for (pr_url, participants) in &stats.participants {
            if let Err(err) = self.db.upsert_pr_participants(pr_url, participants) {
                eprintln!("⚠️ Failed to persist participants for {pr_url}: {err}");
            }
        }

        Ok(stats)
    }

    pub async fn mark_done(&self, request: MarkDoneRequest) -> anyhow::Result<()> {
        let mut marked_any = false;

        if let Some(thread_id) = request.github_thread_id.as_deref() {
            github::mark_notification_done(thread_id).await?;
            self.db.mark_thread_done_local(thread_id)?;
            marked_any = true;
        }

        if request.mark_authored_pr {
            let pr_url = request
                .pr_url
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("Missing PR URL for authored PR done action"))?;
            self.db.mark_authored_pr_done_local(pr_url)?;
            marked_any = true;
        }

        anyhow::ensure!(marked_any, "No done action requested");
        self.notify_dashboard("Marked item done");
        Ok(())
    }

    pub async fn mark_notification_read(&self, thread_id: &str) -> anyhow::Result<()> {
        github::mark_notification_read(thread_id).await?;
        self.db.mark_thread_read_local(thread_id)?;
        self.notify_dashboard("Marked notification read");
        Ok(())
    }

    pub async fn run_fix(
        &self,
        owner: String,
        repo: String,
        number: i64,
    ) -> anyhow::Result<String> {
        let provider = self.config.ai.provider;
        let model = self.config.ai.model.clone();
        let pr_url = format!("https://github.com/{owner}/{repo}/pull/{number}");
        let latest_review = self
            .db
            .latest_review_by_url(&pr_url)?
            .ok_or_else(|| anyhow::anyhow!("No review found for {pr_url}"))?;

        let repo_dir = github::ensure_local_repo(&owner, &repo).await?;
        github::checkout_pr(&repo_dir, &pr_url).await?;

        let agent = provider.as_agent();
        let output = review::run_fix(
            &repo_dir,
            &pr_url,
            &latest_review.content_md,
            Some(&agent),
            model.as_deref(),
        )
        .await;

        match output {
            Ok(text) => {
                self.db
                    .insert_fix_run(&pr_url, provider.as_str(), "success", &text)?;
                self.notify_dashboard(format!("Fix run completed for {pr_url}"));
                Ok(text)
            }
            Err(err) => {
                self.db
                    .insert_fix_run(&pr_url, provider.as_str(), "error", &err.to_string())?;
                self.notify_dashboard(format!("Fix run failed for {pr_url}: {err}"));
                Err(err)
            }
        }
    }

    pub async fn run_review(&self, owner: String, repo: String, number: i64) -> anyhow::Result<()> {
        let _guard = self.poll_lock.lock().await;
        let pr_url = format!("https://github.com/{owner}/{repo}/pull/{number}");
        println!("🔍 Review started: {pr_url}");

        let result = async {
            let details = github::fetch_pr_details(&pr_url).await?;
            upsert_pr_from_details(&self.db, &details)?;
            run_review_for_details(&self.db, &self.config, &self.work_dir, &details).await
        }
        .await;

        match &result {
            Ok(()) => {
                println!("✅ Review finished: {pr_url}");
                self.notify_dashboard(format!("Review finished for {pr_url}"));
            }
            Err(err) => {
                eprintln!("❌ Review failed: {pr_url}: {err}");
                self.notify_dashboard(format!("Review failed for {pr_url}: {err}"));
            }
        }

        result
    }

    pub async fn open_in_vscode(
        &self,
        repository: String,
        pr_url: Option<String>,
    ) -> anyhow::Result<()> {
        let target_label = describe_open_target(&repository, pr_url.as_deref());
        println!("🧑‍💻 VS Code open requested: {target_label}");
        let repo_dir = resolve_open_target_repo(&repository, pr_url.as_deref()).await?;
        println!("📂 Opening VS Code in {repo_dir}");
        let result = launcher::open_vscode(&repo_dir).await;

        match &result {
            Ok(()) => {
                println!("✅ VS Code opened: {target_label}");
                self.notify_dashboard(format!("Opened VS Code for {target_label}"));
            }
            Err(err) => {
                eprintln!("❌ Failed to open VS Code for {target_label}: {err}");
                self.notify_dashboard(format!("Failed to open VS Code for {target_label}: {err}"));
            }
        }

        result
    }

    pub async fn open_in_terminal(
        &self,
        repository: String,
        pr_url: Option<String>,
    ) -> anyhow::Result<()> {
        let target_label = describe_open_target(&repository, pr_url.as_deref());
        println!("🖥️ Terminal open requested: {target_label}");
        let repo_dir = resolve_open_target_repo(&repository, pr_url.as_deref()).await?;
        println!("📂 Opening Terminal in {repo_dir}");
        let result = launcher::open_terminal(&repo_dir).await;

        match &result {
            Ok(()) => {
                println!("✅ Terminal opened: {target_label}");
                self.notify_dashboard(format!("Opened Terminal for {target_label}"));
            }
            Err(err) => {
                eprintln!("❌ Failed to open Terminal for {target_label}: {err}");
                self.notify_dashboard(format!("Failed to open Terminal for {target_label}: {err}"));
            }
        }

        result
    }
}
