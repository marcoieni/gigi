use super::{
    helpers::{dashboard_browser_url, parse_repository_name},
    poll::{
        apply_startup_review_limits, next_incremental_cursor, should_review_pr,
        sync_assigned_issue_threads, sync_assigned_pr_threads, sync_authored_pr_threads,
    },
    time::parse_github_timestamp_to_unix_seconds,
    *,
};
use crate::{config, db};

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
        merge_queue_state: None,
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
        is_draft: false,
    };
    let stored = db::StoredPr {
        pr_url: "u".to_string(),
        owner: "o".to_string(),
        repo: "r".to_string(),
        number: 1,
        state: "OPEN".to_string(),
        merge_queue_state: None,
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
        config::RereviewMode::OnUpdate,
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
        merge_queue_state: None,
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
        is_draft: false,
    };
    let stored = db::StoredPr {
        pr_url: "u".to_string(),
        owner: "o".to_string(),
        repo: "r".to_string(),
        number: 1,
        state: "OPEN".to_string(),
        merge_queue_state: None,
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
        config::RereviewMode::Manual,
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
fn ipv6_host_is_wrapped_in_brackets_in_browser_url() {
    let cfg = AppConfig {
        dashboard: crate::config::DashboardConfig {
            host: "::1".to_string(),
            port: 8787,
        },
        ..AppConfig::default()
    };

    assert_eq!(dashboard_browser_url(&cfg), "http://[::1]:8787");
}

#[test]
fn parses_repository_name() {
    assert_eq!(
        parse_repository_name("marcoieni/gigi").unwrap(),
        ("marcoieni".to_string(), "gigi".to_string())
    );
}

#[test]
fn parses_repository_name_with_surrounding_whitespace() {
    assert_eq!(
        parse_repository_name("  marcoieni / gigi  ").unwrap(),
        ("marcoieni".to_string(), "gigi".to_string())
    );
}

#[test]
fn rejects_invalid_repository_name() {
    assert!(parse_repository_name("marcoieni").is_err());
    assert!(parse_repository_name("marcoieni/gigi/extra").is_err());
    assert!(parse_repository_name("marco ieni/gigi").is_err());
    assert!(parse_repository_name("marcoieni/gi gi").is_err());
}

#[test]
fn parses_github_timestamps() {
    let ts = parse_github_timestamp_to_unix_seconds("1970-01-01T00:00:00Z").unwrap();
    assert_eq!(ts, 0);

    let with_offset = parse_github_timestamp_to_unix_seconds("2026-01-10T02:30:00+02:00").unwrap();
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
            merge_queue_state: None,
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
            is_draft: false,
        },
        github::PrDetails {
            pr_url: "https://github.com/o/r/pull/2".to_string(),
            owner: "o".to_string(),
            repo: "r".to_string(),
            number: 2,
            state: "OPEN".to_string(),
            merge_queue_state: None,
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
            is_draft: false,
        },
        github::PrDetails {
            pr_url: "https://github.com/o/r/pull/3".to_string(),
            owner: "o".to_string(),
            repo: "r".to_string(),
            number: 3,
            state: "OPEN".to_string(),
            merge_queue_state: None,
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
            is_draft: false,
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
        discussion_answered: None,
        reason: Some("authored".to_string()),
        pr_url: Some(stale_pr_url.clone()),
        unread: false,
        done: false,
        updated_at: "2026-01-01T00:00:00Z".to_string(),
        is_draft: false,
    })
    .unwrap();

    let closed_pr = github::AuthoredPrSummary {
        pr_url: stale_pr_url,
        repository: "o/r".to_string(),
        title: "stale".to_string(),
        updated_at: "2026-01-02T00:00:00Z".to_string(),
        is_open: false,
        is_draft: false,
    };
    let current_pr = github::AuthoredPrSummary {
        pr_url: "https://github.com/o/r/pull/2".to_string(),
        repository: "o/r".to_string(),
        title: "current".to_string(),
        updated_at: "2026-01-02T00:00:00Z".to_string(),
        is_open: true,
        is_draft: false,
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
        is_draft: false,
    };

    sync_authored_pr_threads(&db, std::slice::from_ref(&current_pr)).unwrap();
    db.mark_authored_pr_done_local(&current_pr.pr_url).unwrap();
    sync_authored_pr_threads(&db, std::slice::from_ref(&current_pr)).unwrap();

    let threads = db.list_dashboard_threads().unwrap();
    assert!(threads.is_empty());
}

#[test]
fn sync_assigned_pr_threads_removes_stale_entries() {
    let db = test_db();
    let stale_pr_url = "https://github.com/o/r/pull/1".to_string();
    db.upsert_thread(&db::NewThread {
        thread_key: format!("assignedpr:{stale_pr_url}"),
        github_thread_id: None,
        source: "assigned_pr".to_string(),
        repository: "o/r".to_string(),
        subject_type: Some("PullRequest".to_string()),
        subject_title: "stale".to_string(),
        subject_url: Some(stale_pr_url),
        issue_state: None,
        discussion_answered: None,
        reason: Some("assigned".to_string()),
        pr_url: Some("https://github.com/o/r/pull/1".to_string()),
        unread: false,
        done: false,
        updated_at: "2026-01-01T00:00:00Z".to_string(),
        is_draft: false,
    })
    .unwrap();

    let current_pr = github::AssignedPrSummary {
        pr_url: "https://github.com/o/r/pull/2".to_string(),
        repository: "o/r".to_string(),
        title: "current".to_string(),
        updated_at: "2026-01-02T00:00:00Z".to_string(),
        is_draft: false,
    };

    sync_assigned_pr_threads(&db, std::slice::from_ref(&current_pr)).unwrap();

    let threads = db.list_dashboard_threads().unwrap();
    assert_eq!(threads.len(), 1);
    assert_eq!(
        threads[0].pr_url.as_deref(),
        Some(current_pr.pr_url.as_str())
    );
    assert_eq!(threads[0].subject_title, "current");
}

#[test]
fn sync_assigned_pr_threads_preserves_done_entries() {
    let db = test_db();
    let current_pr = github::AssignedPrSummary {
        pr_url: "https://github.com/o/r/pull/2".to_string(),
        repository: "o/r".to_string(),
        title: "current".to_string(),
        updated_at: "2026-01-02T00:00:00Z".to_string(),
        is_draft: false,
    };

    sync_assigned_pr_threads(&db, std::slice::from_ref(&current_pr)).unwrap();
    assert!(db.mark_assigned_pr_done_local(&current_pr.pr_url).unwrap());
    sync_assigned_pr_threads(&db, std::slice::from_ref(&current_pr)).unwrap();

    let threads = db.list_dashboard_threads().unwrap();
    assert!(threads.is_empty());
}

#[test]
fn sync_assigned_issue_threads_removes_stale_entries() {
    let db = test_db();
    let stale_issue_url = "https://github.com/o/r/issues/1".to_string();
    db.upsert_thread(&db::NewThread {
        thread_key: format!("myissue:{stale_issue_url}"),
        github_thread_id: None,
        source: "my_issue".to_string(),
        repository: "o/r".to_string(),
        subject_type: Some("Issue".to_string()),
        subject_title: "stale".to_string(),
        subject_url: Some(stale_issue_url),
        issue_state: Some("OPEN".to_string()),
        discussion_answered: None,
        reason: Some("assigned".to_string()),
        pr_url: None,
        unread: false,
        done: false,
        updated_at: "2026-01-01T00:00:00Z".to_string(),
        is_draft: false,
    })
    .unwrap();

    let current_issue = github::AssignedIssuesSearchResult {
        issues: vec![github::AssignedIssueSummary {
            issue_url: "https://github.com/o/r/issues/2".to_string(),
            repository: "o/r".to_string(),
            title: "current".to_string(),
            updated_at: "2026-01-02T00:00:00Z".to_string(),
            state: "OPEN".to_string(),
        }],
        is_complete: true,
    };

    sync_assigned_issue_threads(&db, &current_issue).unwrap();

    let threads = db.list_dashboard_threads().unwrap();
    assert_eq!(threads.len(), 1);
    assert_eq!(
        threads[0].subject_url.as_deref(),
        Some(current_issue.issues[0].issue_url.as_str())
    );
    assert_eq!(threads[0].subject_title, "current");
}

#[test]
fn sync_assigned_issue_threads_preserves_done_entries() {
    let db = test_db();
    let current_issue = github::AssignedIssuesSearchResult {
        issues: vec![github::AssignedIssueSummary {
            issue_url: "https://github.com/o/r/issues/2".to_string(),
            repository: "o/r".to_string(),
            title: "current".to_string(),
            updated_at: "2026-01-02T00:00:00Z".to_string(),
            state: "OPEN".to_string(),
        }],
        is_complete: true,
    };

    sync_assigned_issue_threads(&db, &current_issue).unwrap();
    assert!(
        db.mark_assigned_issue_done_local(&current_issue.issues[0].issue_url)
            .unwrap()
    );
    sync_assigned_issue_threads(&db, &current_issue).unwrap();

    let threads = db.list_dashboard_threads().unwrap();
    assert!(threads.is_empty());
}

#[test]
fn mark_done_rejects_missing_assigned_issue_row() {
    let db = test_db();
    let work_dir = camino::Utf8PathBuf::from_path_buf(std::env::temp_dir()).unwrap();
    let (dashboard_updates, _) = tokio::sync::watch::channel(DashboardUpdate {
        version: 0,
        message: String::new(),
    });
    let state = AppState {
        db,
        config: config::AppConfig::default(),
        work_dir,
        poll_lock: std::sync::Arc::new(tokio::sync::Mutex::new(())),
        dashboard_refresh_in_flight: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        dashboard_updates,
    };

    let err = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(state.mark_done(MarkDoneRequest {
            github_thread_id: None,
            pr_url: None,
            subject_url: Some("https://github.com/o/r/issues/404".to_string()),
            mark_authored_pr: false,
            mark_assigned_pr: false,
            mark_assigned_issue: true,
        }))
        .unwrap_err();

    assert_eq!(err.to_string(), "No done action requested");
}

#[test]
fn sync_assigned_issue_threads_skips_stale_deletes_for_incomplete_results() {
    let db = test_db();
    let stale_issue_url = "https://github.com/o/r/issues/1".to_string();
    db.upsert_thread(&db::NewThread {
        thread_key: format!("myissue:{stale_issue_url}"),
        github_thread_id: None,
        source: "my_issue".to_string(),
        repository: "o/r".to_string(),
        subject_type: Some("Issue".to_string()),
        subject_title: "stale".to_string(),
        subject_url: Some(stale_issue_url.clone()),
        issue_state: Some("OPEN".to_string()),
        discussion_answered: None,
        reason: Some("assigned".to_string()),
        pr_url: None,
        unread: false,
        done: false,
        updated_at: "2026-01-01T00:00:00Z".to_string(),
        is_draft: false,
    })
    .unwrap();

    let current_issue = github::AssignedIssuesSearchResult {
        issues: vec![github::AssignedIssueSummary {
            issue_url: "https://github.com/o/r/issues/2".to_string(),
            repository: "o/r".to_string(),
            title: "current".to_string(),
            updated_at: "2026-01-02T00:00:00Z".to_string(),
            state: "OPEN".to_string(),
        }],
        is_complete: false,
    };

    sync_assigned_issue_threads(&db, &current_issue).unwrap();

    let threads = db.list_dashboard_threads().unwrap();
    assert_eq!(threads.len(), 2);
    assert!(threads.iter().any(|thread| thread.subject_title == "stale"));
    assert!(
        threads
            .iter()
            .any(|thread| thread.subject_title == "current")
    );
}

#[test]
fn next_incremental_cursor_uses_overlap_from_newest_seen_timestamp() {
    let next = next_incremental_cursor(
        Some("2026-01-10T10:00:00Z"),
        parse_github_timestamp_to_unix_seconds("2026-01-10T10:08:00Z"),
        "2026-01-10T10:09:00Z",
    );

    assert_eq!(next, "2026-01-10T10:03:00Z");
}

#[test]
fn next_incremental_cursor_does_not_move_backwards_when_results_are_stale() {
    let next = next_incremental_cursor(
        Some("2026-01-10T10:00:00Z"),
        parse_github_timestamp_to_unix_seconds("2026-01-10T10:02:00Z"),
        "2026-01-10T10:09:00Z",
    );

    assert_eq!(next, "2026-01-10T10:00:00Z");
}

#[test]
fn next_incremental_cursor_keeps_previous_cursor_when_no_results_arrive() {
    let next = next_incremental_cursor(Some("2026-01-10T10:00:00Z"), None, "2026-01-10T10:09:00Z");

    assert_eq!(next, "2026-01-10T10:00:00Z");
}

#[test]
fn next_incremental_cursor_bootstraps_from_now_when_cursor_is_missing() {
    let next = next_incremental_cursor(None, None, "2026-01-10T10:09:00Z");

    assert_eq!(next, "2026-01-10T10:04:00Z");
}
