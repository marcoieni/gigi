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
    let mut path = std::env::temp_dir();
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    path.push(format!("gigi-test-{ts}.sqlite"));
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
