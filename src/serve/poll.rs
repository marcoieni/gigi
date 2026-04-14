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

// Keep a small overlap between polls so late-arriving GitHub updates are re-fetched
// instead of being skipped forever by a strict "since last seen timestamp" cursor.
const FETCH_CURSOR_OVERLAP_SECS: i64 = 300;

pub(super) async fn poll_once_async(
    db: &Db,
    config: &AppConfig,
    work_dir: &Utf8Path,
    mode: PollMode,
) -> anyhow::Result<PollStats> {
    let notification_cursor = db.get_kv("last_notifications_fetch")?;
    let notification_fetch_since = notification_cursor.as_deref();
    let notification_now = poll_cursor_now();
    println!(
        "🔎 Notification fetch: mode={mode:?} stored_since={} request_since={}",
        notification_cursor.as_deref().unwrap_or("<none>"),
        notification_fetch_since.unwrap_or("<none>")
    );
    let mut notifications = github::fetch_notifications(notification_fetch_since).await?;
    print_fetched_notifications(&notifications);
    let newest_notification_ts = newest_seen_timestamp(
        notifications
            .iter()
            .map(|notification| notification.updated_at.as_str()),
    );
    let next_notification_cursor = next_incremental_cursor(
        notification_cursor.as_deref(),
        newest_notification_ts,
        &notification_now,
    );
    println!(
        "⏱️ Cursor advance [notifications]: previous={} newest_seen={} next={}",
        notification_cursor.as_deref().unwrap_or("<none>"),
        format_cursor_debug_value(newest_notification_ts),
        next_notification_cursor
    );

    let authored_pr_cursor = db.get_kv("last_authored_prs_fetch")?;
    let authored_pr_fetch_since = authored_pr_cursor.as_deref();
    let authored_pr_now = poll_cursor_now();
    println!(
        "🔎 Authored PR fetch: mode={mode:?} stored_since={} request_since={}",
        authored_pr_cursor.as_deref().unwrap_or("<none>"),
        authored_pr_fetch_since.unwrap_or("<none>")
    );
    let authored_prs = github::fetch_authored_prs(authored_pr_fetch_since).await?;
    print_fetched_authored_prs(&authored_prs);
    let newest_authored_pr_ts =
        newest_seen_timestamp(authored_prs.iter().map(|pr| pr.updated_at.as_str()));
    let next_authored_pr_cursor = next_incremental_cursor(
        authored_pr_cursor.as_deref(),
        newest_authored_pr_ts,
        &authored_pr_now,
    );
    println!(
        "⏱️ Cursor advance [authored_prs]: previous={} newest_seen={} next={}",
        authored_pr_cursor.as_deref().unwrap_or("<none>"),
        format_cursor_debug_value(newest_authored_pr_ts),
        next_authored_pr_cursor
    );
    sync_authored_pr_threads(db, &authored_prs)?;

    println!("🔎 Assigned PR fetch: mode={mode:?}");
    let assigned_prs = github::fetch_assigned_prs().await?;
    print_fetched_assigned_prs(&assigned_prs);
    sync_assigned_pr_threads(db, &assigned_prs)?;

    println!("🔎 Assigned issue fetch: mode={mode:?}");
    let assigned_issues = github::fetch_assigned_issues().await?;
    print_fetched_assigned_issues(&assigned_issues);
    sync_assigned_issue_threads(db, &assigned_issues)?;

    let mut pr_urls = HashSet::new();
    for notification in &notifications {
        if let Some(pr_url) = &notification.pr_url {
            pr_urls.insert(pr_url.clone());
        }
    }
    for authored in &authored_prs {
        pr_urls.insert(authored.pr_url.clone());
    }
    for assigned in &assigned_prs {
        pr_urls.insert(assigned.pr_url.clone());
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

    db.set_kv("last_notifications_fetch", &next_notification_cursor)?;
    db.set_kv("last_authored_prs_fetch", &next_authored_pr_cursor)?;

    Ok(PollStats {
        notifications_fetched: notifications.len(),
        authored_prs_fetched: authored_prs.len(),
        assigned_prs_fetched: assigned_prs.len(),
        assigned_issues_fetched: assigned_issues.issues.len(),
        prs_seen: pr_urls.len(),
        reviews_run,
        participants: batch.participants,
    })
}

fn poll_cursor_now() -> String {
    chrono::Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)
}

fn newest_seen_timestamp<I, S>(fetched_updated_ats: I) -> Option<i64>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    fetched_updated_ats
        .into_iter()
        .filter_map(|updated_at| parse_github_timestamp_to_unix_seconds(updated_at.as_ref()))
        .max()
}

/// Advances the incremental fetch cursor without trusting wall-clock `now`.
///
/// We base the next cursor on the newest `updated_at` returned by GitHub, then move it
/// slightly backward by `FETCH_CURSOR_OVERLAP_SECS`. That overlap makes incremental
/// polling resilient to delayed indexing or responses that arrive out of order while
/// still preventing the cursor from moving backwards.
pub(super) fn next_incremental_cursor(
    previous_cursor: Option<&str>,
    newest_seen_ts: Option<i64>,
    fallback_now: &str,
) -> String {
    let previous_ts = previous_cursor.and_then(parse_github_timestamp_to_unix_seconds);
    let fallback_now_ts = parse_github_timestamp_to_unix_seconds(fallback_now);

    let next_ts = if let Some(newest_seen_ts) = newest_seen_ts {
        let candidate = newest_seen_ts.saturating_sub(FETCH_CURSOR_OVERLAP_SECS);
        previous_ts.map_or(candidate, |previous_ts| previous_ts.max(candidate))
    } else if let Some(previous_ts) = previous_ts {
        previous_ts
    } else if let Some(fallback_now_ts) = fallback_now_ts {
        fallback_now_ts.saturating_sub(FETCH_CURSOR_OVERLAP_SECS)
    } else {
        return fallback_now.to_string();
    };

    unix_seconds_to_github_timestamp(next_ts).unwrap_or_else(|| fallback_now.to_string())
}

fn unix_seconds_to_github_timestamp(unix_seconds: i64) -> Option<String> {
    chrono::DateTime::from_timestamp(unix_seconds, 0)
        .map(|ts| ts.to_rfc3339_opts(SecondsFormat::Secs, true))
}

fn format_cursor_debug_value(unix_seconds: Option<i64>) -> String {
    unix_seconds
        .and_then(unix_seconds_to_github_timestamp)
        .unwrap_or_else(|| "<none>".to_string())
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

pub(crate) fn sync_assigned_issue_threads(
    db: &Db,
    assigned_issues: &github::AssignedIssuesSearchResult,
) -> anyhow::Result<()> {
    let open_issue_urls: Vec<_> = assigned_issues
        .issues
        .iter()
        .filter(|issue| issue.state == "OPEN")
        .map(|issue| issue.issue_url.clone())
        .collect();

    if assigned_issues.is_complete {
        println!(
            "🗄️ DB delete threads: source=my_issue keep_open_issue_urls={}",
            open_issue_urls.len()
        );
        db.delete_threads_by_source_except_subject_urls("my_issue", &open_issue_urls)?;
    } else {
        println!(
            "🗄️ DB skip delete threads: source=my_issue reason=incomplete_search_results keep_open_issue_urls={}",
            open_issue_urls.len()
        );
    }

    for issue in assigned_issues
        .issues
        .iter()
        .filter(|issue| issue.state == "OPEN")
    {
        let thread_key = format!("myissue:{}", issue.issue_url);
        let row = db::NewThread {
            thread_key,
            github_thread_id: None,
            source: "my_issue".to_string(),
            repository: issue.repository.clone(),
            subject_type: Some("Issue".to_string()),
            subject_title: issue.title.clone(),
            subject_url: Some(issue.issue_url.clone()),
            issue_state: Some(issue.state.clone()),
            discussion_answered: None,
            reason: Some("assigned".to_string()),
            pr_url: None,
            unread: false,
            done: false,
            updated_at: issue.updated_at.clone(),
            is_draft: false,
        };
        db.upsert_thread(&row)?;
        print_thread_db_write(&row);
    }

    Ok(())
}

pub(crate) fn sync_assigned_pr_threads(
    db: &Db,
    assigned_prs: &[github::AssignedPrSummary],
) -> anyhow::Result<()> {
    let open_pr_urls: Vec<_> = assigned_prs.iter().map(|pr| pr.pr_url.clone()).collect();

    println!(
        "🗄️ DB delete threads: source=assigned_pr keep_open_pr_urls={}",
        open_pr_urls.len()
    );
    db.delete_threads_by_source_except_pr_urls("assigned_pr", &open_pr_urls)?;

    for assigned in assigned_prs {
        let thread_key = format!("assignedpr:{}", assigned.pr_url);
        let row = db::NewThread {
            thread_key,
            github_thread_id: None,
            source: "assigned_pr".to_string(),
            repository: assigned.repository.clone(),
            subject_type: Some("PullRequest".to_string()),
            subject_title: assigned.title.clone(),
            subject_url: Some(assigned.pr_url.clone()),
            issue_state: None,
            discussion_answered: None,
            reason: Some("assigned".to_string()),
            pr_url: Some(assigned.pr_url.clone()),
            unread: false,
            done: false,
            updated_at: assigned.updated_at.clone(),
            is_draft: assigned.is_draft,
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

fn print_fetched_assigned_issues(assigned_issues: &github::AssignedIssuesSearchResult) {
    println!(
        "📥 Assigned issues fetched: {} complete={}",
        assigned_issues.issues.len(),
        assigned_issues.is_complete
    );
    for issue in &assigned_issues.issues {
        println!(
            "  • issue_url={} repo={} state={} updated_at={} title={}",
            issue.issue_url, issue.repository, issue.state, issue.updated_at, issue.title
        );
    }
}

fn print_fetched_assigned_prs(assigned_prs: &[github::AssignedPrSummary]) {
    println!("📥 Assigned PRs fetched: {}", assigned_prs.len());
    for assigned in assigned_prs {
        println!(
            "  • pr_url={} repo={} updated_at={} is_draft={} title={}",
            assigned.pr_url,
            assigned.repository,
            assigned.updated_at,
            assigned.is_draft,
            assigned.title
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
        "{prefix} notifications={}, my_prs={}, assigned_prs={}, assigned_issues={}, prs={}, reviews={}",
        stats.notifications_fetched,
        stats.authored_prs_fetched,
        stats.assigned_prs_fetched,
        stats.assigned_issues_fetched,
        stats.prs_seen,
        stats.reviews_run
    );
}
