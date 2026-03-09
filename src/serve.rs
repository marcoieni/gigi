use std::{
    collections::HashSet,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::Context as _;
use camino::{Utf8Path, Utf8PathBuf};
use serde::Serialize;

use crate::{
    config::{self, AiProvider, AppConfig, RereviewMode},
    db::{self, Db},
    github, launcher, review, web,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PollMode {
    Startup,
    Regular,
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

pub async fn run_serve() -> anyhow::Result<()> {
    let paths = config::resolve_paths()?;
    config::ensure_parent_dirs(&paths).await?;

    let cfg = config::load_config(&paths.config_path).await?;
    let db = Db::new(&paths.db_path)?;
    web::prepare_dashboard_assets(&paths.dashboard_dir).await?;

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

    let startup_state = Arc::clone(&state);
    let startup_handle = tokio::spawn(async move {
        println!("🔄 Starting initial poll cycle...");
        match startup_state.poll_once_startup().await {
            Ok(stats) => print_poll_stats("✅ Initial poll complete:", &stats),
            Err(err) => eprintln!("⚠️ Initial poll cycle failed: {err}"),
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
        server_result = web::run_server(state, &cfg, &paths.dashboard_dir) => {
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

fn print_poll_stats(prefix: &str, stats: &PollStats) {
    println!(
        "{prefix} notifications={}, my_prs={}, prs={}, reviews={}",
        stats.notifications_fetched, stats.authored_prs_fetched, stats.prs_seen, stats.reviews_run
    );
}

impl AppState {
    pub async fn poll_once_from_dashboard(&self) -> anyhow::Result<PollStats> {
        println!("🔄 Dashboard refresh requested");
        let result = self.poll_once_regular().await;
        match &result {
            Ok(stats) => print_poll_stats("✅ Dashboard refresh complete:", stats),
            Err(err) => eprintln!("❌ Dashboard refresh failed: {err}"),
        }
        result
    }

    pub async fn poll_once_startup(&self) -> anyhow::Result<PollStats> {
        self.poll_once_with_mode(PollMode::Startup).await
    }

    pub async fn poll_once_regular(&self) -> anyhow::Result<PollStats> {
        self.poll_once_with_mode(PollMode::Regular).await
    }

    async fn poll_once_with_mode(&self, mode: PollMode) -> anyhow::Result<PollStats> {
        let _guard = self.poll_lock.lock().await;

        poll_once_async(&self.db, &self.config, &self.work_dir, mode)
            .await
            .context("polling cycle failed")
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
                    .insert_fix_run(&pr_url, provider_name(provider), "success", &text)?;
                Ok(text)
            }
            Err(err) => {
                self.db.insert_fix_run(
                    &pr_url,
                    provider_name(provider),
                    "error",
                    &err.to_string(),
                )?;
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
            Ok(()) => println!("✅ Review finished: {pr_url}"),
            Err(err) => eprintln!("❌ Review failed: {pr_url}: {err}"),
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
            Ok(()) => println!("✅ VS Code opened: {target_label}"),
            Err(err) => eprintln!("❌ Failed to open VS Code for {target_label}: {err}"),
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
            Ok(()) => println!("✅ Terminal opened: {target_label}"),
            Err(err) => eprintln!("❌ Failed to open Terminal for {target_label}: {err}"),
        }

        result
    }
}

#[derive(Debug, Clone)]
pub struct MarkDoneRequest {
    pub github_thread_id: Option<String>,
    pub pr_url: Option<String>,
    pub mark_authored_pr: bool,
}

async fn poll_once_async(
    db: &Db,
    config: &AppConfig,
    work_dir: &Utf8Path,
    mode: PollMode,
) -> anyhow::Result<PollStats> {
    let since = db.get_kv("last_notifications_fetch")?;
    let now = chrono::Utc::now().to_rfc3339();
    let mut notifications = github::fetch_notifications(since.as_deref()).await?;
    db.set_kv("last_notifications_fetch", &now)?;
    print_fetched_notifications(&notifications);

    let authored_prs_since = db.get_kv("last_authored_prs_fetch")?;
    let authored_prs_now = chrono::Utc::now().to_rfc3339();
    let authored_prs = github::fetch_authored_prs(authored_prs_since.as_deref()).await?;
    db.set_kv("last_authored_prs_fetch", &authored_prs_now)?;
    print_fetched_authored_prs(&authored_prs);
    sync_authored_pr_threads(db, &authored_prs)?;

    // Collect all PR URLs and issue API URLs for batch fetch.
    let mut pr_urls = HashSet::new();
    for n in &notifications {
        if let Some(pr_url) = &n.pr_url {
            pr_urls.insert(pr_url.clone());
        }
    }
    for authored in &authored_prs {
        pr_urls.insert(authored.pr_url.clone());
    }

    let issue_api_urls: Vec<String> = notifications
        .iter()
        .filter_map(|n| n.issue_api_url.clone())
        .collect();

    let pr_url_list: Vec<String> = pr_urls.iter().cloned().collect();
    let batch = github::fetch_batch(&pr_url_list, &issue_api_urls).await?;

    // Fill in issue states on notifications from batch result.
    for n in &mut notifications {
        if let Some(api_url) = &n.issue_api_url {
            n.issue_state = batch.issue_states.get(api_url).cloned();
        }
    }

    for notification in &notifications {
        let thread_key = format!("notif:{}", notification.thread_id);
        let row = db::NewThread {
            thread_key,
            github_thread_id: Some(notification.thread_id.clone()),
            source: "notification".to_string(),
            repository: notification.repository.clone(),
            subject_type: notification.subject_type.clone(),
            subject_title: notification.subject_title.clone(),
            subject_url: notification.subject_url.clone(),
            issue_state: notification.issue_state.clone(),
            reason: notification.reason.clone(),
            pr_url: notification.pr_url.clone(),
            unread: notification.unread,
            done: false,
            updated_at: notification.updated_at.clone(),
        };
        db.upsert_thread(&row)?;
        print_thread_db_write(&row);
    }

    let startup_limits = (mode == PollMode::Startup).then_some(StartupReviewLimits {
        lookback_days: config.initial_review_lookback_days,
        max_prs: config.initial_review_max_prs,
    });

    let mut reviews_run = 0_usize;
    let mut review_candidates = Vec::new();

    for pr_url in &pr_urls {
        let details = match batch.pr_details.get(pr_url) {
            Some(details) => details.clone(),
            None => {
                eprintln!("⚠️ No PR details from batch for {pr_url}");
                continue;
            }
        };

        print_pr_details("Fetched PR details", &details);
        let stored = db.get_pr(&details.pr_url)?;
        upsert_pr_from_details(db, &details)?;
        print_pr_db_write(stored.as_ref(), &details);

        if should_review_pr(config.rereview_mode, stored.as_ref(), &details) {
            review_candidates.push(details.clone());
        }

        if details.state != "OPEN"
            && let Err(err) = handle_closed_pr_branch_sync(db, &details).await
        {
            eprintln!(
                "⚠️ Failed to process closed PR branch sync for {}: {err}",
                details.pr_url
            );
            drop(db.insert_sync_event(&details.pr_url, "error", &err.to_string()));
        }
    }

    let selection = if let Some(limits) = startup_limits {
        apply_startup_review_limits(review_candidates, limits, unix_ts())
    } else {
        StartupReviewSelection {
            to_review: review_candidates,
            to_mark_baseline: Vec::new(),
        }
    };

    for details in selection.to_mark_baseline {
        db.set_pr_review_marker(&details.pr_url, &details.head_sha, &details.updated_at)?;
    }

    for details in selection.to_review {
        println!("🔍 Auto-review started: {}", details.pr_url);
        match run_review_for_details(db, config, work_dir, &details).await {
            Ok(()) => {
                println!("✅ Auto-review finished: {}", details.pr_url);
                reviews_run += 1;
            }
            Err(err) => {
                eprintln!("❌ Auto-review failed: {}: {err}", details.pr_url);
            }
        }
    }

    Ok(PollStats {
        notifications_fetched: notifications.len(),
        authored_prs_fetched: authored_prs.len(),
        prs_seen: pr_urls.len(),
        reviews_run,
    })
}

/// Keeps the `threads` table in sync with the user's authored PRs.
///
/// The dashboard shows threads from two sources: GitHub notifications and the
/// user's own PRs (`source = "my_pr"`). GitHub notifications alone aren't
/// enough because they only appear when someone else interacts with a PR —
/// a freshly opened PR with no comments would be invisible on the dashboard.
///
/// `authored_prs` contains all PRs updated since the last fetch (any state).
/// This function:
/// 1. Deletes `my_pr` threads for PRs that are now closed/merged.
/// 2. Upserts a thread for every still-open PR, so the dashboard always
///    lists all of the user's active PRs regardless of notification state.
fn sync_authored_pr_threads(
    db: &Db,
    authored_prs: &[github::AuthoredPrSummary],
) -> anyhow::Result<()> {
    let closed_pr_urls: Vec<_> = authored_prs
        .iter()
        .filter(|pr| !pr.is_open)
        .map(|pr| pr.pr_url.clone())
        .collect();

    println!(
        "🗄️ DB delete threads: source=my_pr closed_pr_urls={}",
        closed_pr_urls.len()
    );
    db.delete_threads_by_source_and_pr_urls("my_pr", &closed_pr_urls)?;

    for authored in authored_prs.iter().filter(|pr| pr.is_open) {
        let thread_key = format!("mypr:{}", authored.pr_url);
        let row = db::NewThread {
            thread_key,
            github_thread_id: None,
            source: "my_pr".to_string(),
            repository: authored.repository.clone(),
            subject_type: Some("PullRequest".to_string()),
            subject_title: authored.title.clone(),
            subject_url: Some(authored.pr_url.clone()),
            issue_state: None,
            reason: Some("authored".to_string()),
            pr_url: Some(authored.pr_url.clone()),
            unread: false,
            done: false,
            updated_at: authored.updated_at.clone(),
        };
        db.upsert_thread(&row)?;
        print_thread_db_write(&row);
    }

    Ok(())
}

fn upsert_pr_from_details(db: &Db, details: &github::PrDetails) -> anyhow::Result<()> {
    let row = db::NewPr {
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
        is_archived: details.is_archived,
    };
    db.upsert_pr(&row)
}

fn print_fetched_notifications(notifications: &[github::NotificationThread]) {
    println!("📥 Notifications fetched: {}", notifications.len());
    for notification in notifications {
        println!(
            "  • thread={} repo={} type={} unread={} reason={} pr_url={} updated_at={} title={}",
            notification.thread_id,
            notification.repository,
            notification.subject_type.as_deref().unwrap_or("<unknown>"),
            notification.unread,
            notification.reason.as_deref().unwrap_or("<none>"),
            notification.pr_url.as_deref().unwrap_or("<none>"),
            notification.updated_at,
            notification.subject_title
        );
    }
}

fn print_fetched_authored_prs(authored_prs: &[github::AuthoredPrSummary]) {
    println!("📥 Authored open PRs fetched: {}", authored_prs.len());
    for authored in authored_prs {
        println!(
            "  • pr_url={} repo={} updated_at={} title={}",
            authored.pr_url, authored.repository, authored.updated_at, authored.title
        );
    }
}

fn print_pr_details(prefix: &str, details: &github::PrDetails) {
    println!(
        "📥 {prefix}: pr_url={} state={} head_sha={} updated_at={} archived={} title={}",
        details.pr_url,
        details.state,
        details.head_sha,
        details.updated_at,
        details.is_archived,
        details.title
    );
}

fn print_thread_db_write(row: &db::NewThread) {
    println!(
        "🗄️ DB upsert thread [{}]: source={} repo={} pr_url={} unread={} done={} updated_at={} title={}",
        row.thread_key,
        row.source,
        row.repository,
        row.pr_url.as_deref().unwrap_or("<none>"),
        row.unread,
        row.done,
        row.updated_at,
        row.subject_title
    );
}

fn print_pr_db_write(existing: Option<&db::StoredPr>, details: &github::PrDetails) {
    let action = match existing {
        None => "insert",
        Some(_) => "update",
    };
    println!(
        "🗄️ DB {action} pr [{}]: state={} head_sha={} updated_at={} archived={} title={}",
        details.pr_url,
        details.state,
        details.head_sha,
        details.updated_at,
        details.is_archived,
        details.title
    );
}

async fn run_review_for_details(
    db: &Db,
    config: &AppConfig,
    work_dir: &Utf8Path,
    details: &github::PrDetails,
) -> anyhow::Result<()> {
    let agent = config.ai.provider.as_agent();
    let review_result = review::generate_review(
        work_dir,
        &details.pr_url,
        Some(&agent),
        config.ai.model.as_deref(),
    )
    .await?;

    db.insert_review(&db::NewReview {
        pr_url: details.pr_url.clone(),
        provider: review_result.provider,
        model: review_result.model,
        requires_code_changes: review_result.requires_code_changes,
        content_md: review_result.markdown,
    })?;
    db.set_pr_review_marker(&details.pr_url, &details.head_sha, &details.updated_at)?;

    Ok(())
}

fn apply_startup_review_limits(
    candidates: Vec<github::PrDetails>,
    limits: StartupReviewLimits,
    now_unix_seconds: i64,
) -> StartupReviewSelection {
    let lookback_seconds = i64::try_from(limits.lookback_days)
        .unwrap_or(i64::MAX)
        .saturating_mul(86_400);
    let cutoff = now_unix_seconds.saturating_sub(lookback_seconds);

    let mut recent = Vec::new();
    let mut to_mark_baseline = Vec::new();

    for details in candidates {
        if is_pr_recent_enough(&details, cutoff) {
            recent.push(details);
        } else {
            to_mark_baseline.push(details);
        }
    }

    recent.sort_by_key(|b| std::cmp::Reverse(pr_timestamp_for_sort(b)));

    let mut to_review = Vec::new();
    for details in recent {
        if to_review.len() < limits.max_prs {
            to_review.push(details);
        } else {
            to_mark_baseline.push(details);
        }
    }

    StartupReviewSelection {
        to_review,
        to_mark_baseline,
    }
}

fn is_pr_recent_enough(details: &github::PrDetails, cutoff_unix_seconds: i64) -> bool {
    let created_recent = parse_github_timestamp_to_unix_seconds(&details.created_at)
        .is_some_and(|ts| ts >= cutoff_unix_seconds);
    let updated_recent = parse_github_timestamp_to_unix_seconds(&details.updated_at)
        .is_some_and(|ts| ts >= cutoff_unix_seconds);

    created_recent || updated_recent
}

fn pr_timestamp_for_sort(details: &github::PrDetails) -> i64 {
    parse_github_timestamp_to_unix_seconds(&details.updated_at).unwrap_or(0)
}

fn parse_github_timestamp_to_unix_seconds(timestamp: &str) -> Option<i64> {
    let (date_part, time_part) = timestamp.split_once('T')?;
    let (year, month, day) = parse_date_parts(date_part)?;
    let (hour, minute, second, tz_offset_seconds) = parse_time_and_offset(time_part)?;

    let days = days_from_civil(year, month, day)?;
    let day_seconds = i64::from(hour) * 3600 + i64::from(minute) * 60 + i64::from(second);
    days.saturating_mul(86_400)
        .checked_add(day_seconds)
        .and_then(|local_seconds| local_seconds.checked_sub(i64::from(tz_offset_seconds)))
}

fn parse_date_parts(date_part: &str) -> Option<(i32, u32, u32)> {
    let mut parts = date_part.split('-');
    let year = parts.next()?.parse::<i32>().ok()?;
    let month = parts.next()?.parse::<u32>().ok()?;
    let day = parts.next()?.parse::<u32>().ok()?;
    if parts.next().is_some() {
        return None;
    }
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    Some((year, month, day))
}

fn parse_time_and_offset(time_part: &str) -> Option<(u32, u32, u32, i32)> {
    if let Some(clock) = time_part.strip_suffix('Z') {
        let (hour, minute, second) = parse_hms(clock)?;
        return Some((hour, minute, second, 0));
    }

    let tz_pos = time_part.rfind(['+', '-'])?;
    let (clock, offset_part) = time_part.split_at(tz_pos);
    let sign = if offset_part.starts_with('-') { -1 } else { 1 };
    let offset = &offset_part[1..];
    let (offset_hour, offset_minute) = parse_hm(offset)?;
    let tz_offset_seconds = sign * (offset_hour * 3600 + offset_minute * 60);
    let (hour, minute, second) = parse_hms(clock)?;
    Some((hour, minute, second, tz_offset_seconds))
}

fn parse_hms(clock: &str) -> Option<(u32, u32, u32)> {
    let mut parts = clock.split(':');
    let hour = parts.next()?.parse::<u32>().ok()?;
    let minute = parts.next()?.parse::<u32>().ok()?;
    let second_raw = parts.next()?;
    if parts.next().is_some() {
        return None;
    }

    let second_text = second_raw
        .split_once('.')
        .map_or(second_raw, |(sec, _)| sec);
    let second = second_text.parse::<u32>().ok()?;
    if hour > 23 || minute > 59 || second > 59 {
        return None;
    }
    Some((hour, minute, second))
}

fn parse_hm(clock: &str) -> Option<(i32, i32)> {
    let mut parts = clock.split(':');
    let hour = parts.next()?.parse::<i32>().ok()?;
    let minute = parts.next()?.parse::<i32>().ok()?;
    if parts.next().is_some() {
        return None;
    }
    if !(0..=23).contains(&hour) || !(0..=59).contains(&minute) {
        return None;
    }
    Some((hour, minute))
}

fn days_from_civil(year: i32, month: u32, day: u32) -> Option<i64> {
    let adjusted_year = year - i32::from(month <= 2);
    let era = if adjusted_year >= 0 {
        adjusted_year / 400
    } else {
        (adjusted_year - 399) / 400
    };
    let yoe = adjusted_year - era * 400;
    let month_i32 = i32::try_from(month).ok()?;
    let day_i32 = i32::try_from(day).ok()?;
    let m = month_i32 + if month > 2 { -3 } else { 9 };
    let doy = (153 * m + 2) / 5 + day_i32 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(i64::from(era) * 146_097 + i64::from(doe) - 719_468)
}

fn unix_ts() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
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

async fn handle_closed_pr_branch_sync(db: &Db, details: &github::PrDetails) -> anyhow::Result<()> {
    let repo_dir = github::local_repo_dir(&details.owner, &details.repo)?;
    if !repo_dir.exists() || !repo_dir.join(".git").exists() {
        return Ok(());
    }

    let current_branch = github::current_branch(&repo_dir).await?;
    if current_branch != details.head_ref {
        return Ok(());
    }

    if !github::is_clean_repo(&repo_dir).await? {
        let message = format!(
            "Skipped sync for {} because working tree is dirty on branch '{}'.",
            details.pr_url, details.head_ref
        );
        db.insert_sync_event(&details.pr_url, "warning", &message)?;
        return Ok(());
    }

    let default_branch = github::default_branch(&repo_dir).await?;
    github::checkout_branch(&repo_dir, &default_branch).await?;
    github::pull_ff_only(&repo_dir).await?;

    let message = format!(
        "Switched to default branch '{default_branch}' and pulled latest changes after PR closed."
    );
    db.insert_sync_event(&details.pr_url, "success", &message)?;

    Ok(())
}

fn parse_repository_name(repository: &str) -> anyhow::Result<(String, String)> {
    let Some((owner, repo)) = repository.split_once('/') else {
        anyhow::bail!("Invalid repository name '{repository}' (expected owner/repo)");
    };
    anyhow::ensure!(
        !owner.is_empty() && !repo.is_empty() && !repo.contains('/'),
        "Invalid repository name '{repository}' (expected owner/repo)"
    );
    Ok((owner.to_string(), repo.to_string()))
}

async fn resolve_open_target_repo(
    repository: &str,
    pr_url: Option<&str>,
) -> anyhow::Result<Utf8PathBuf> {
    if let Some(pr_url) = pr_url {
        let local_pr = github::ensure_local_repo_for_pr(pr_url).await?;
        println!(
            "🔀 Preparing PR for open action: {}",
            local_pr.details.pr_url
        );
        github::checkout_pr_for_open_with_details(&local_pr.repo_dir, &local_pr.details).await?;
        return Ok(local_pr.repo_dir);
    }

    let (owner, repo) = parse_repository_name(repository)?;
    github::ensure_local_repo(&owner, &repo).await
}

fn describe_open_target(repository: &str, pr_url: Option<&str>) -> String {
    match pr_url {
        Some(pr_url) => format!("{repository} ({pr_url})"),
        None => repository.to_string(),
    }
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

    fn test_db() -> Db {
        let mut path = std::env::temp_dir();
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        path.push(format!("gigi-serve-test-{ts}.sqlite"));
        Db::new(path).unwrap()
    }

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
            created_at: "2025-12-31T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            is_archived: false,
            author_login: None,
            head_repo_owner: None,
            head_repo_name: None,
            is_cross_repository: false,
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
            is_archived: false,
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
            created_at: "2025-12-31T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            is_archived: false,
            author_login: None,
            head_repo_owner: None,
            head_repo_name: None,
            is_cross_repository: false,
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
            is_archived: false,
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

    #[test]
    fn parses_repository_name() {
        assert_eq!(
            parse_repository_name("marcoieni/gigi").unwrap(),
            ("marcoieni".to_string(), "gigi".to_string())
        );
    }

    #[test]
    fn rejects_invalid_repository_name() {
        assert!(parse_repository_name("marcoieni").is_err());
        assert!(parse_repository_name("marcoieni/gigi/extra").is_err());
    }

    #[test]
    fn parses_github_timestamps() {
        let ts = parse_github_timestamp_to_unix_seconds("1970-01-01T00:00:00Z").unwrap();
        assert_eq!(ts, 0);

        let with_offset =
            parse_github_timestamp_to_unix_seconds("2026-01-10T02:30:00+02:00").unwrap();
        let utc = parse_github_timestamp_to_unix_seconds("2026-01-10T00:30:00Z").unwrap();
        assert_eq!(with_offset, utc);
    }

    #[test]
    fn startup_limits_filter_and_cap_reviews() {
        let now = parse_github_timestamp_to_unix_seconds("2026-01-10T00:00:00Z").unwrap();
        let limits = StartupReviewLimits {
            lookback_days: 3,
            max_prs: 1,
        };
        let candidates = vec![
            github::PrDetails {
                pr_url: "https://github.com/o/r/pull/1".to_string(),
                owner: "o".to_string(),
                repo: "r".to_string(),
                number: 1,
                state: "OPEN".to_string(),
                title: "old".to_string(),
                head_ref: "feat1".to_string(),
                base_ref: "main".to_string(),
                head_sha: "sha1".to_string(),
                created_at: "2025-12-01T00:00:00Z".to_string(),
                updated_at: "2026-01-01T00:00:00Z".to_string(),
                is_archived: false,
                author_login: None,
                head_repo_owner: None,
                head_repo_name: None,
                is_cross_repository: false,
            },
            github::PrDetails {
                pr_url: "https://github.com/o/r/pull/2".to_string(),
                owner: "o".to_string(),
                repo: "r".to_string(),
                number: 2,
                state: "OPEN".to_string(),
                title: "recent".to_string(),
                head_ref: "feat2".to_string(),
                base_ref: "main".to_string(),
                head_sha: "sha2".to_string(),
                created_at: "2026-01-09T00:00:00Z".to_string(),
                updated_at: "2026-01-09T12:00:00Z".to_string(),
                is_archived: false,
                author_login: None,
                head_repo_owner: None,
                head_repo_name: None,
                is_cross_repository: false,
            },
            github::PrDetails {
                pr_url: "https://github.com/o/r/pull/3".to_string(),
                owner: "o".to_string(),
                repo: "r".to_string(),
                number: 3,
                state: "OPEN".to_string(),
                title: "recent newer".to_string(),
                head_ref: "feat3".to_string(),
                base_ref: "main".to_string(),
                head_sha: "sha3".to_string(),
                created_at: "2026-01-09T00:00:00Z".to_string(),
                updated_at: "2026-01-09T20:00:00Z".to_string(),
                is_archived: false,
                author_login: None,
                head_repo_owner: None,
                head_repo_name: None,
                is_cross_repository: false,
            },
        ];

        let selected = apply_startup_review_limits(candidates, limits, now);
        assert_eq!(selected.to_review.len(), 1);
        assert_eq!(selected.to_review[0].number, 3);
        assert_eq!(selected.to_mark_baseline.len(), 2);
    }

    #[test]
    fn sync_authored_pr_threads_removes_stale_entries() {
        let db = test_db();
        let stale_pr_url = "https://github.com/o/r/pull/1".to_string();
        db.upsert_thread(&db::NewThread {
            thread_key: format!("mypr:{stale_pr_url}"),
            github_thread_id: None,
            source: "my_pr".to_string(),
            repository: "o/r".to_string(),
            subject_type: Some("PullRequest".to_string()),
            subject_title: "stale".to_string(),
            subject_url: Some(stale_pr_url.clone()),
            issue_state: None,
            reason: Some("authored".to_string()),
            pr_url: Some(stale_pr_url.clone()),
            unread: false,
            done: false,
            updated_at: "2026-01-01T00:00:00Z".to_string(),
        })
        .unwrap();

        let closed_pr = github::AuthoredPrSummary {
            pr_url: stale_pr_url,
            repository: "o/r".to_string(),
            title: "stale".to_string(),
            updated_at: "2026-01-02T00:00:00Z".to_string(),
            is_open: false,
        };
        let current_pr = github::AuthoredPrSummary {
            pr_url: "https://github.com/o/r/pull/2".to_string(),
            repository: "o/r".to_string(),
            title: "current".to_string(),
            updated_at: "2026-01-02T00:00:00Z".to_string(),
            is_open: true,
        };

        sync_authored_pr_threads(&db, &[closed_pr, current_pr.clone()]).unwrap();

        let threads = db.list_dashboard_threads().unwrap();
        assert_eq!(threads.len(), 1);
        assert_eq!(
            threads[0].pr_url.as_deref(),
            Some(current_pr.pr_url.as_str())
        );
        assert_eq!(threads[0].subject_title, "current");
    }

    #[test]
    fn sync_authored_pr_threads_preserves_done_entries() {
        let db = test_db();
        let current_pr = github::AuthoredPrSummary {
            pr_url: "https://github.com/o/r/pull/2".to_string(),
            repository: "o/r".to_string(),
            title: "current".to_string(),
            updated_at: "2026-01-02T00:00:00Z".to_string(),
            is_open: true,
        };

        sync_authored_pr_threads(&db, std::slice::from_ref(&current_pr)).unwrap();
        db.mark_authored_pr_done_local(&current_pr.pr_url).unwrap();
        sync_authored_pr_threads(&db, std::slice::from_ref(&current_pr)).unwrap();

        let threads = db.list_dashboard_threads().unwrap();
        assert!(threads.is_empty());
    }
}
