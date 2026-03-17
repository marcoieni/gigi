use std::collections::HashSet;

use camino::Utf8Path;
use chrono::SecondsFormat;

use crate::{
    config::{AppConfig, RereviewMode},
    db::{self, Db},
    github, review,
};

use super::{
    PollMode, PollStats, StartupReviewLimits, StartupReviewSelection,
    time::{parse_github_timestamp_to_unix_seconds, unix_ts},
};

pub(super) async fn poll_once_async(
    db: &Db,
    config: &AppConfig,
    work_dir: &Utf8Path,
    mode: PollMode,
) -> anyhow::Result<PollStats> {
    let since = db.get_kv("last_notifications_fetch")?;
    let now = poll_cursor_now();
    let mut notifications = github::fetch_notifications(since.as_deref()).await?;
    db.set_kv("last_notifications_fetch", &now)?;
    print_fetched_notifications(&notifications);

    let authored_prs_since = db.get_kv("last_authored_prs_fetch")?;
    let authored_prs_now = poll_cursor_now();
    let authored_prs = github::fetch_authored_prs(authored_prs_since.as_deref()).await?;
    db.set_kv("last_authored_prs_fetch", &authored_prs_now)?;
    print_fetched_authored_prs(&authored_prs);
    sync_authored_pr_threads(db, &authored_prs)?;

    let mut pr_urls = HashSet::new();
    for notification in &notifications {
        if let Some(pr_url) = &notification.pr_url {
            pr_urls.insert(pr_url.clone());
        }
    }
    for authored in &authored_prs {
        pr_urls.insert(authored.pr_url.clone());
    }

    let issue_api_urls: Vec<String> = notifications
        .iter()
        .filter_map(|notification| notification.issue_api_url.clone())
        .collect();
    let discussion_api_urls: Vec<String> = notifications
        .iter()
        .filter_map(|notification| notification.discussion_api_url.clone())
        .collect();

    let pr_url_list: Vec<String> = pr_urls.iter().cloned().collect();
    let batch = github::fetch_batch(&pr_url_list, &issue_api_urls, &discussion_api_urls).await?;

    for notification in &mut notifications {
        if let Some(api_url) = &notification.issue_api_url {
            notification.issue_state = batch.issue_states.get(api_url).cloned();
        }
        if let Some(api_url) = &notification.discussion_api_url {
            notification.issue_state = batch.discussion_states.get(api_url).cloned();
            notification.discussion_answered = batch.discussion_answers.get(api_url).copied();
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
            discussion_answered: notification.discussion_answered,
            reason: notification.reason.clone(),
            pr_url: notification.pr_url.clone(),
            unread: notification.unread,
            done: false,
            updated_at: notification.updated_at.clone(),
            is_draft: false,
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
        participants: batch.participants,
    })
}

fn poll_cursor_now() -> String {
    chrono::Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)
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
pub(crate) fn sync_authored_pr_threads(
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
            discussion_answered: None,
            reason: Some("authored".to_string()),
            pr_url: Some(authored.pr_url.clone()),
            unread: false,
            done: false,
            updated_at: authored.updated_at.clone(),
            is_draft: authored.is_draft,
        };
        db.upsert_thread(&row)?;
        print_thread_db_write(&row);
    }

    Ok(())
}

pub(super) fn upsert_pr_from_details(db: &Db, details: &github::PrDetails) -> anyhow::Result<()> {
    let row = db::NewPr {
        pr_url: details.pr_url.clone(),
        owner: details.owner.clone(),
        repo: details.repo.clone(),
        number: details.number,
        state: details.state.clone(),
        merge_queue_state: details.merge_queue_state.clone(),
        title: details.title.clone(),
        head_ref: details.head_ref.clone(),
        base_ref: details.base_ref.clone(),
        head_sha: details.head_sha.clone(),
        updated_at: details.updated_at.clone(),
        is_archived: details.is_archived,
        is_draft: details.is_draft,
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
            "  • pr_url={} repo={} updated_at={} is_draft={} title={}",
            authored.pr_url,
            authored.repository,
            authored.updated_at,
            authored.is_draft,
            authored.title
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

pub(super) async fn run_review_for_details(
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

pub(crate) fn apply_startup_review_limits(
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

    recent.sort_by_key(|details| std::cmp::Reverse(pr_timestamp_for_sort(details)));

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
        .is_some_and(|timestamp| timestamp >= cutoff_unix_seconds);
    let updated_recent = parse_github_timestamp_to_unix_seconds(&details.updated_at)
        .is_some_and(|timestamp| timestamp >= cutoff_unix_seconds);

    created_recent || updated_recent
}

fn pr_timestamp_for_sort(details: &github::PrDetails) -> i64 {
    parse_github_timestamp_to_unix_seconds(&details.updated_at).unwrap_or(0)
}

pub(crate) fn should_review_pr(
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

pub(super) fn print_poll_stats(prefix: &str, stats: &PollStats) {
    println!(
        "{prefix} notifications={}, my_prs={}, prs={}, reviews={}",
        stats.notifications_fetched, stats.authored_prs_fetched, stats.prs_seen, stats.reviews_run
    );
}
