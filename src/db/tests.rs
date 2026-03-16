use std::sync::atomic::{AtomicU64, Ordering};

use rusqlite::params;

use super::*;

impl Db {
    pub fn latest_review_for_pr(
        &self,
        owner: &str,
        repo: &str,
        number: i64,
    ) -> anyhow::Result<Option<StoredReview>> {
        let pr_url = format!("https://github.com/{owner}/{repo}/pull/{number}");
        self.latest_review_by_url(&pr_url)
    }
}

fn test_db() -> Db {
    static NEXT_DB_ID: AtomicU64 = AtomicU64::new(1);

    let mut path = std::env::temp_dir();
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    // avoid parallel test collision
    let id = NEXT_DB_ID.fetch_add(1, Ordering::Relaxed);
    path.push(format!("gigi-test-{ts}-{id}.sqlite"));
    Db::new(path).unwrap()
}

#[test]
fn upsert_thread_is_idempotent() {
    let db = test_db();
    let row = NewThread {
        is_draft: false,
        thread_key: "thread-1".to_string(),
        github_thread_id: Some("1".to_string()),
        source: "notification".to_string(),
        repository: "a/b".to_string(),
        subject_type: Some("PullRequest".to_string()),
        subject_title: "t".to_string(),
        subject_url: Some("u".to_string()),
        issue_state: None,
        reason: Some("review_requested".to_string()),
        pr_url: Some("https://github.com/a/b/pull/1".to_string()),
        unread: true,
        done: false,
        updated_at: "2026-01-01T00:00:00Z".to_string(),
    };

    db.upsert_thread(&row).unwrap();
    db.upsert_thread(&row).unwrap();

    let threads = db.list_dashboard_threads().unwrap();
    assert_eq!(threads.len(), 1);
}

#[test]
fn latest_review_roundtrip() {
    let db = test_db();

    db.upsert_pr(&NewPr {
        pr_url: "https://github.com/a/b/pull/1".to_string(),
        owner: "a".to_string(),
        repo: "b".to_string(),
        number: 1,
        state: "OPEN".to_string(),
        merge_queue_state: None,
        title: "Title".to_string(),
        head_ref: "feat".to_string(),
        base_ref: "main".to_string(),
        head_sha: "sha1".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
        is_archived: false,
        is_draft: false,
    })
    .unwrap();

    db.insert_review(&NewReview {
        pr_url: "https://github.com/a/b/pull/1".to_string(),
        provider: "copilot".to_string(),
        model: None,
        requires_code_changes: true,
        content_md: "review".to_string(),
    })
    .unwrap();

    let review = db.latest_review_for_pr("a", "b", 1).unwrap().unwrap();
    assert!(review.requires_code_changes);
    assert_eq!(review.content_md, "review");
}

#[test]
fn insert_review_strips_control_sequences() {
    let db = test_db();

    db.upsert_pr(&NewPr {
        pr_url: "https://github.com/a/b/pull/1".to_string(),
        owner: "a".to_string(),
        repo: "b".to_string(),
        number: 1,
        state: "OPEN".to_string(),
        merge_queue_state: None,
        title: "Title".to_string(),
        head_ref: "feat".to_string(),
        base_ref: "main".to_string(),
        head_sha: "sha1".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
        is_archived: false,
        is_draft: false,
    })
    .unwrap();

    db.insert_review(&NewReview {
        pr_url: "https://github.com/a/b/pull/1".to_string(),
        provider: "copilot".to_string(),
        model: None,
        requires_code_changes: false,
        content_md: "\u{1b}[38;5;141mSummary\u{1b}[0m".to_string(),
    })
    .unwrap();

    let review = db.latest_review_for_pr("a", "b", 1).unwrap().unwrap();
    assert_eq!(review.content_md, "Summary");
}

#[test]
fn db_init_cleans_existing_reviews() {
    let mut path = std::env::temp_dir();
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    path.push(format!("gigi-test-clean-{ts}.sqlite"));

    {
        let db = Db::new(&path).unwrap();
        db.upsert_pr(&NewPr {
            pr_url: "https://github.com/a/b/pull/1".to_string(),
            owner: "a".to_string(),
            repo: "b".to_string(),
            number: 1,
            state: "OPEN".to_string(),
            merge_queue_state: None,
            title: "Title".to_string(),
            head_ref: "feat".to_string(),
            base_ref: "main".to_string(),
            head_sha: "sha1".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            is_archived: false,
            is_draft: false,
        })
        .unwrap();
        db.with_conn(|conn| {
            conn.execute(
                "INSERT INTO reviews (pr_url, provider, model, requires_code_changes, content_md, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    "https://github.com/a/b/pull/1",
                    "copilot",
                    Option::<String>::None,
                    0_i64,
                    "\u{1b}[38;5;141mSummary\u{1b}[0m",
                    0_i64,
                ],
            )?;
            Ok(())
        })
        .unwrap();
    }

    let db = Db::new(&path).unwrap();
    let review = db.latest_review_for_pr("a", "b", 1).unwrap().unwrap();
    assert_eq!(review.content_md, "Summary");
}

#[test]
fn insert_review_prefers_requires_code_changes_header() {
    let db = test_db();

    db.upsert_pr(&NewPr {
        pr_url: "https://github.com/a/b/pull/1".to_string(),
        owner: "a".to_string(),
        repo: "b".to_string(),
        number: 1,
        state: "OPEN".to_string(),
        merge_queue_state: None,
        title: "Title".to_string(),
        head_ref: "feat".to_string(),
        base_ref: "main".to_string(),
        head_sha: "sha1".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
        is_archived: false,
        is_draft: false,
    })
    .unwrap();

    db.insert_review(&NewReview {
        pr_url: "https://github.com/a/b/pull/1".to_string(),
        provider: "copilot".to_string(),
        model: None,
        requires_code_changes: true,
        content_md: "REQUIRES_CODE_CHANGES: NO\nSummary".to_string(),
    })
    .unwrap();

    let review = db.latest_review_for_pr("a", "b", 1).unwrap().unwrap();
    assert!(!review.requires_code_changes);
}

#[test]
fn db_init_repairs_stale_requires_code_changes() {
    let mut path = std::env::temp_dir();
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    path.push(format!("gigi-test-repair-{ts}.sqlite"));

    {
        let db = Db::new(&path).unwrap();
        db.upsert_pr(&NewPr {
            pr_url: "https://github.com/a/b/pull/1".to_string(),
            owner: "a".to_string(),
            repo: "b".to_string(),
            number: 1,
            state: "OPEN".to_string(),
            merge_queue_state: None,
            title: "Title".to_string(),
            head_ref: "feat".to_string(),
            base_ref: "main".to_string(),
            head_sha: "sha1".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            is_archived: false,
            is_draft: false,
        })
        .unwrap();
        db.upsert_thread(&NewThread {
            is_draft: false,
            thread_key: "thread-1".to_string(),
            github_thread_id: Some("1".to_string()),
            source: "notification".to_string(),
            repository: "a/b".to_string(),
            subject_type: Some("PullRequest".to_string()),
            subject_title: "t".to_string(),
            subject_url: Some("u".to_string()),
            issue_state: None,
            reason: Some("review_requested".to_string()),
            pr_url: Some("https://github.com/a/b/pull/1".to_string()),
            unread: true,
            done: false,
            updated_at: "2026-01-01T00:00:00Z".to_string(),
        })
        .unwrap();
        db.with_conn(|conn| {
            conn.execute(
                "INSERT INTO reviews (pr_url, provider, model, requires_code_changes, content_md, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    "https://github.com/a/b/pull/1",
                    "copilot",
                    Option::<String>::None,
                    1_i64,
                    "REQUIRES_CODE_CHANGES: NO\nSummary",
                    0_i64,
                ],
            )?;
            Ok(())
        })
        .unwrap();
    }

    let db = Db::new(&path).unwrap();
    let review = db.latest_review_for_pr("a", "b", 1).unwrap().unwrap();
    assert!(!review.requires_code_changes);

    let threads = db.list_dashboard_threads().unwrap();
    assert_eq!(threads.len(), 1);
    assert_eq!(threads[0].latest_requires_code_changes, Some(false));
}

#[test]
fn done_threads_are_hidden_from_dashboard() {
    let db = test_db();
    let row = NewThread {
        is_draft: false,
        thread_key: "thread-1".to_string(),
        github_thread_id: Some("1".to_string()),
        source: "notification".to_string(),
        repository: "a/b".to_string(),
        subject_type: Some("PullRequest".to_string()),
        subject_title: "t".to_string(),
        subject_url: Some("u".to_string()),
        issue_state: None,
        reason: Some("review_requested".to_string()),
        pr_url: Some("https://github.com/a/b/pull/1".to_string()),
        unread: true,
        done: false,
        updated_at: "2026-01-01T00:00:00Z".to_string(),
    };

    db.upsert_thread(&row).unwrap();
    db.mark_thread_done_local("1").unwrap();

    let threads = db.list_dashboard_threads().unwrap();
    assert!(threads.is_empty());
}

#[test]
fn done_threads_stay_done_after_upsert() {
    let db = test_db();
    let row = NewThread {
        is_draft: false,
        thread_key: "thread-1".to_string(),
        github_thread_id: Some("1".to_string()),
        source: "notification".to_string(),
        repository: "a/b".to_string(),
        subject_type: Some("PullRequest".to_string()),
        subject_title: "t".to_string(),
        subject_url: Some("u".to_string()),
        issue_state: None,
        reason: Some("review_requested".to_string()),
        pr_url: Some("https://github.com/a/b/pull/1".to_string()),
        unread: true,
        done: false,
        updated_at: "2026-01-01T00:00:00Z".to_string(),
    };

    db.upsert_thread(&row).unwrap();
    db.mark_thread_done_local("1").unwrap();
    db.upsert_thread(&row).unwrap();

    let done = db
        .with_conn(|conn| {
            conn.query_row(
                "SELECT done FROM threads WHERE thread_key = ?1",
                [&row.thread_key],
                |row| row.get::<_, i64>(0),
            )
            .map_err(anyhow::Error::from)
        })
        .unwrap();

    assert_eq!(done, 1);
}

#[test]
fn done_authored_prs_are_hidden_from_dashboard() {
    let db = test_db();
    let pr_url = "https://github.com/a/b/pull/1".to_string();
    let row = NewThread {
        is_draft: false,
        thread_key: format!("mypr:{pr_url}"),
        github_thread_id: None,
        source: "my_pr".to_string(),
        repository: "a/b".to_string(),
        subject_type: Some("PullRequest".to_string()),
        subject_title: "t".to_string(),
        subject_url: Some(pr_url.clone()),
        issue_state: None,
        reason: Some("authored".to_string()),
        pr_url: Some(pr_url.clone()),
        unread: false,
        done: false,
        updated_at: "2026-01-01T00:00:00Z".to_string(),
    };

    db.upsert_thread(&row).unwrap();
    db.mark_authored_pr_done_local(&pr_url).unwrap();

    let threads = db.list_dashboard_threads().unwrap();
    assert!(threads.is_empty());
}

#[test]
fn done_authored_prs_stay_done_after_upsert() {
    let db = test_db();
    let pr_url = "https://github.com/a/b/pull/1".to_string();
    let row = NewThread {
        is_draft: false,
        thread_key: format!("mypr:{pr_url}"),
        github_thread_id: None,
        source: "my_pr".to_string(),
        repository: "a/b".to_string(),
        subject_type: Some("PullRequest".to_string()),
        subject_title: "t".to_string(),
        subject_url: Some(pr_url.clone()),
        issue_state: None,
        reason: Some("authored".to_string()),
        pr_url: Some(pr_url.clone()),
        unread: false,
        done: false,
        updated_at: "2026-01-01T00:00:00Z".to_string(),
    };

    db.upsert_thread(&row).unwrap();
    db.mark_authored_pr_done_local(&pr_url).unwrap();
    db.upsert_thread(&row).unwrap();

    let done = db
        .with_conn(|conn| {
            conn.query_row(
                "SELECT done FROM threads WHERE thread_key = ?1",
                [&row.thread_key],
                |row| row.get::<_, i64>(0),
            )
            .map_err(anyhow::Error::from)
        })
        .unwrap();

    assert_eq!(done, 1);
}

#[test]
fn dashboard_can_show_done_items_when_requested() {
    let db = test_db();
    let row = NewThread {
        is_draft: false,
        thread_key: "thread-1".to_string(),
        github_thread_id: Some("1".to_string()),
        source: "notification".to_string(),
        repository: "a/b".to_string(),
        subject_type: Some("PullRequest".to_string()),
        subject_title: "t".to_string(),
        subject_url: Some("u".to_string()),
        issue_state: None,
        reason: Some("review_requested".to_string()),
        pr_url: Some("https://github.com/a/b/pull/1".to_string()),
        unread: false,
        done: false,
        updated_at: "2026-01-01T00:00:00Z".to_string(),
    };

    db.upsert_thread(&row).unwrap();
    db.mark_thread_done_local("1").unwrap();

    let threads = db
        .list_dashboard_threads_with_filters(&DashboardThreadFilters {
            show_notifications: true,
            show_prs: true,
            show_done: true,
            show_not_done: false,
            group_by_repository: true,
            hidden_repositories: Vec::new(),
        })
        .unwrap();

    assert_eq!(threads.len(), 1);
    assert!(threads[0].done);
}

#[test]
fn dashboard_filters_by_source_type() {
    let db = test_db();
    let notification_pr_url = "https://github.com/a/b/pull/1".to_string();
    let authored_pr_url = "https://github.com/a/b/pull/2".to_string();

    db.upsert_thread(&NewThread {
        is_draft: false,
        thread_key: "notif:1".to_string(),
        github_thread_id: Some("1".to_string()),
        source: "notification".to_string(),
        repository: "a/b".to_string(),
        subject_type: Some("PullRequest".to_string()),
        subject_title: "notification".to_string(),
        subject_url: Some(notification_pr_url.clone()),
        issue_state: None,
        reason: Some("review_requested".to_string()),
        pr_url: Some(notification_pr_url),
        unread: true,
        done: false,
        updated_at: "2026-01-02T00:00:00Z".to_string(),
    })
    .unwrap();

    db.upsert_thread(&NewThread {
        is_draft: false,
        thread_key: format!("mypr:{authored_pr_url}"),
        github_thread_id: None,
        source: "my_pr".to_string(),
        repository: "a/b".to_string(),
        subject_type: Some("PullRequest".to_string()),
        subject_title: "authored".to_string(),
        subject_url: Some(authored_pr_url.clone()),
        issue_state: None,
        reason: Some("authored".to_string()),
        pr_url: Some(authored_pr_url),
        unread: false,
        done: false,
        updated_at: "2026-01-01T00:00:00Z".to_string(),
    })
    .unwrap();

    let threads = db
        .list_dashboard_threads_with_filters(&DashboardThreadFilters {
            show_notifications: false,
            show_prs: true,
            show_done: false,
            show_not_done: true,
            group_by_repository: true,
            hidden_repositories: Vec::new(),
        })
        .unwrap();

    assert_eq!(threads.len(), 1);
    assert_eq!(threads[0].sources, vec!["my_pr"]);
    assert_eq!(threads[0].subject_title, "authored");
}

#[test]
fn dashboard_filter_preferences_default_when_unset() {
    let db = test_db();

    assert_eq!(
        db.dashboard_thread_filters().unwrap(),
        DashboardThreadFilters::default()
    );
}

#[test]
fn dashboard_filter_preferences_roundtrip() {
    let db = test_db();
    let filters = DashboardThreadFilters {
        show_notifications: false,
        show_prs: true,
        show_done: true,
        show_not_done: false,
        group_by_repository: false,
        hidden_repositories: vec!["a/b".to_string(), "c/d".to_string()],
    };

    db.set_dashboard_thread_filters(&filters).unwrap();
    db.set_repository_filter(&filters.hidden_repositories)
        .unwrap();

    assert_eq!(db.dashboard_thread_filters().unwrap(), filters);
}

#[test]
fn dashboard_threads_deduplicate_notification_and_my_pr() {
    let db = test_db();
    let pr_url = "https://github.com/a/b/pull/1".to_string();

    db.upsert_pr(&NewPr {
        pr_url: pr_url.clone(),
        owner: "a".to_string(),
        repo: "b".to_string(),
        number: 1,
        state: "MERGED".to_string(),
        merge_queue_state: None,
        title: "Title".to_string(),
        head_ref: "feat".to_string(),
        base_ref: "main".to_string(),
        head_sha: "sha1".to_string(),
        updated_at: "2026-01-02T00:00:00Z".to_string(),
        is_archived: false,
        is_draft: false,
    })
    .unwrap();

    db.upsert_thread(&NewThread {
        is_draft: false,
        thread_key: format!("mypr:{pr_url}"),
        github_thread_id: None,
        source: "my_pr".to_string(),
        repository: "a/b".to_string(),
        subject_type: Some("PullRequest".to_string()),
        subject_title: "Authored title".to_string(),
        subject_url: Some(pr_url.clone()),
        issue_state: None,
        reason: Some("authored".to_string()),
        pr_url: Some(pr_url.clone()),
        unread: false,
        done: false,
        updated_at: "2026-01-02T00:00:00Z".to_string(),
    })
    .unwrap();

    db.upsert_thread(&NewThread {
        is_draft: false,
        thread_key: "notif:123".to_string(),
        github_thread_id: Some("123".to_string()),
        source: "notification".to_string(),
        repository: "a/b".to_string(),
        subject_type: Some("PullRequest".to_string()),
        subject_title: "Notification title".to_string(),
        subject_url: Some(pr_url.clone()),
        issue_state: None,
        reason: Some("review_requested".to_string()),
        pr_url: Some(pr_url.clone()),
        unread: true,
        done: false,
        updated_at: "2026-01-01T00:00:00Z".to_string(),
    })
    .unwrap();

    let threads = db.list_dashboard_threads().unwrap();
    assert_eq!(threads.len(), 1);
    assert_eq!(threads[0].github_thread_id.as_deref(), Some("123"));
    assert_eq!(threads[0].thread_key, "notif:123");
    assert_eq!(threads[0].sources, vec!["notification", "my_pr"]);
    assert_eq!(threads[0].subject_title, "Notification title");
    assert_eq!(threads[0].updated_at, "2026-01-02T00:00:00Z");
    assert_eq!(threads[0].pr_state.as_deref(), Some("MERGED"));
    assert!(threads[0].unread);
}

#[test]
fn archived_open_prs_are_hidden_from_dashboard() {
    let db = test_db();
    let pr_url = "https://github.com/a/b/pull/1".to_string();

    db.upsert_pr(&NewPr {
        pr_url: pr_url.clone(),
        owner: "a".to_string(),
        repo: "b".to_string(),
        number: 1,
        state: "OPEN".to_string(),
        merge_queue_state: Some("QUEUED".to_string()),
        title: "Title".to_string(),
        head_ref: "feat".to_string(),
        base_ref: "main".to_string(),
        head_sha: "sha1".to_string(),
        updated_at: "2026-01-02T00:00:00Z".to_string(),
        is_archived: true,
        is_draft: false,
    })
    .unwrap();

    db.upsert_thread(&NewThread {
        is_draft: false,
        thread_key: format!("mypr:{pr_url}"),
        github_thread_id: None,
        source: "my_pr".to_string(),
        repository: "a/b".to_string(),
        subject_type: Some("PullRequest".to_string()),
        subject_title: "Authored title".to_string(),
        subject_url: Some(pr_url.clone()),
        issue_state: None,
        reason: Some("authored".to_string()),
        pr_url: Some(pr_url),
        unread: false,
        done: false,
        updated_at: "2026-01-02T00:00:00Z".to_string(),
    })
    .unwrap();

    let threads = db.list_dashboard_threads().unwrap();
    assert!(threads.is_empty());
}

#[test]
fn archived_closed_prs_remain_visible_on_dashboard() {
    let db = test_db();
    let pr_url = "https://github.com/a/b/pull/1".to_string();

    db.upsert_pr(&NewPr {
        pr_url: pr_url.clone(),
        owner: "a".to_string(),
        repo: "b".to_string(),
        number: 1,
        state: "MERGED".to_string(),
        merge_queue_state: None,
        title: "Title".to_string(),
        head_ref: "feat".to_string(),
        base_ref: "main".to_string(),
        head_sha: "sha1".to_string(),
        updated_at: "2026-01-02T00:00:00Z".to_string(),
        is_archived: true,
        is_draft: false,
    })
    .unwrap();

    db.upsert_thread(&NewThread {
        is_draft: false,
        thread_key: format!("mypr:{pr_url}"),
        github_thread_id: None,
        source: "my_pr".to_string(),
        repository: "a/b".to_string(),
        subject_type: Some("PullRequest".to_string()),
        subject_title: "Authored title".to_string(),
        subject_url: Some(pr_url.clone()),
        issue_state: None,
        reason: Some("authored".to_string()),
        pr_url: Some(pr_url),
        unread: false,
        done: false,
        updated_at: "2026-01-02T00:00:00Z".to_string(),
    })
    .unwrap();

    let threads = db.list_dashboard_threads().unwrap();
    assert_eq!(threads.len(), 1);
    assert_eq!(threads[0].pr_state.as_deref(), Some("MERGED"));
}

#[test]
fn dashboard_threads_expose_merge_queue_state() {
    let db = test_db();
    let pr_url = "https://github.com/a/b/pull/1".to_string();

    db.upsert_pr(&NewPr {
        pr_url: pr_url.clone(),
        owner: "a".to_string(),
        repo: "b".to_string(),
        number: 1,
        state: "OPEN".to_string(),
        merge_queue_state: Some("QUEUED".to_string()),
        title: "Title".to_string(),
        head_ref: "feat".to_string(),
        base_ref: "main".to_string(),
        head_sha: "sha1".to_string(),
        updated_at: "2026-01-02T00:00:00Z".to_string(),
        is_archived: false,
        is_draft: false,
    })
    .unwrap();

    db.upsert_thread(&NewThread {
        is_draft: false,
        thread_key: format!("mypr:{pr_url}"),
        github_thread_id: None,
        source: "my_pr".to_string(),
        repository: "a/b".to_string(),
        subject_type: Some("PullRequest".to_string()),
        subject_title: "Authored title".to_string(),
        subject_url: Some(pr_url.clone()),
        issue_state: None,
        reason: Some("authored".to_string()),
        pr_url: Some(pr_url),
        unread: false,
        done: false,
        updated_at: "2026-01-02T00:00:00Z".to_string(),
    })
    .unwrap();

    let threads = db.list_dashboard_threads().unwrap();
    assert_eq!(threads.len(), 1);
    assert_eq!(threads[0].pr_merge_queue_state.as_deref(), Some("QUEUED"));
}

#[test]
fn dashboard_threads_expose_issue_state() {
    let db = test_db();

    db.upsert_thread(&NewThread {
        is_draft: false,
        thread_key: "notif:issue-1".to_string(),
        github_thread_id: Some("1".to_string()),
        source: "notification".to_string(),
        repository: "a/b".to_string(),
        subject_type: Some("Issue".to_string()),
        subject_title: "issue".to_string(),
        subject_url: Some("https://github.com/a/b/issues/1".to_string()),
        issue_state: Some("CLOSED".to_string()),
        reason: Some("mention".to_string()),
        pr_url: None,
        unread: true,
        done: false,
        updated_at: "2026-01-02T00:00:00Z".to_string(),
    })
    .unwrap();

    let threads = db.list_dashboard_threads().unwrap();
    assert_eq!(threads.len(), 1);
    assert_eq!(threads[0].issue_state.as_deref(), Some("CLOSED"));
}
