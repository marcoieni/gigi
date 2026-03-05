use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::Context as _;
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;

#[derive(Debug, Clone)]
pub struct Db {
    path: PathBuf,
}

#[derive(Debug, Clone)]
pub struct NewThread {
    pub thread_key: String,
    pub github_thread_id: Option<String>,
    pub source: String,
    pub repository: String,
    pub subject_type: Option<String>,
    pub subject_title: String,
    pub subject_url: Option<String>,
    pub reason: Option<String>,
    pub pr_url: Option<String>,
    pub unread: bool,
    pub done: bool,
    pub updated_at: String,
}

#[derive(Debug, Clone)]
pub struct NewPr {
    pub pr_url: String,
    pub owner: String,
    pub repo: String,
    pub number: i64,
    pub state: String,
    pub title: String,
    pub head_ref: String,
    pub base_ref: String,
    pub head_sha: String,
    pub updated_at: String,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct StoredPr {
    pub pr_url: String,
    pub owner: String,
    pub repo: String,
    pub number: i64,
    pub state: String,
    pub title: String,
    pub head_ref: String,
    pub base_ref: String,
    pub head_sha: String,
    pub updated_at: String,
    pub last_reviewed_sha: Option<String>,
    pub last_reviewed_updated_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct StoredReview {
    pub id: i64,
    pub pr_url: String,
    pub provider: String,
    pub model: Option<String>,
    pub requires_code_changes: bool,
    pub content_md: String,
    pub created_at: i64,
}

#[derive(Debug, Clone)]
pub struct NewReview {
    pub pr_url: String,
    pub provider: String,
    pub model: Option<String>,
    pub requires_code_changes: bool,
    pub content_md: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DashboardThread {
    pub thread_key: String,
    pub github_thread_id: Option<String>,
    pub source: String,
    pub repository: String,
    pub subject_type: Option<String>,
    pub subject_title: String,
    pub subject_url: Option<String>,
    pub reason: Option<String>,
    pub pr_url: Option<String>,
    pub unread: bool,
    pub done: bool,
    pub updated_at: String,
    pub latest_requires_code_changes: Option<bool>,
    pub pr_state: Option<String>,
}

impl Db {
    pub fn new(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).with_context(|| {
                format!("Failed to create DB parent directory {}", parent.display())
            })?;
        }

        let db = Self { path };
        db.with_conn(|conn| {
            run_migrations(conn)?;
            Ok(())
        })?;

        Ok(db)
    }

    pub fn with_conn<T>(
        &self,
        f: impl FnOnce(&Connection) -> anyhow::Result<T>,
    ) -> anyhow::Result<T> {
        let conn = Connection::open(&self.path)
            .with_context(|| format!("Failed to open sqlite DB at {}", self.path.display()))?;
        conn.execute("PRAGMA foreign_keys = ON", [])?;
        f(&conn)
    }

    pub fn upsert_thread(&self, row: &NewThread) -> anyhow::Result<()> {
        let now = unix_ts();
        self.with_conn(|conn| {
            conn.execute(
                r#"
                INSERT INTO threads (
                    thread_key, github_thread_id, source, repository, subject_type, subject_title,
                    subject_url, reason, pr_url, unread, done, updated_at, last_seen_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
                ON CONFLICT(thread_key) DO UPDATE SET
                    github_thread_id = excluded.github_thread_id,
                    source = excluded.source,
                    repository = excluded.repository,
                    subject_type = excluded.subject_type,
                    subject_title = excluded.subject_title,
                    subject_url = excluded.subject_url,
                    reason = excluded.reason,
                    pr_url = excluded.pr_url,
                    unread = excluded.unread,
                    done = MAX(threads.done, excluded.done),
                    updated_at = excluded.updated_at,
                    last_seen_at = excluded.last_seen_at
                "#,
                params![
                    row.thread_key,
                    row.github_thread_id,
                    row.source,
                    row.repository,
                    row.subject_type,
                    row.subject_title,
                    row.subject_url,
                    row.reason,
                    row.pr_url,
                    bool_to_int(row.unread),
                    bool_to_int(row.done),
                    row.updated_at,
                    now,
                ],
            )?;
            Ok(())
        })
    }

    pub fn upsert_pr(&self, row: &NewPr) -> anyhow::Result<()> {
        let now = unix_ts();
        self.with_conn(|conn| {
            conn.execute(
                r#"
                INSERT INTO prs (
                    pr_url, owner, repo, number, state, title, head_ref, base_ref, head_sha,
                    updated_at, last_seen_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
                ON CONFLICT(pr_url) DO UPDATE SET
                    owner = excluded.owner,
                    repo = excluded.repo,
                    number = excluded.number,
                    state = excluded.state,
                    title = excluded.title,
                    head_ref = excluded.head_ref,
                    base_ref = excluded.base_ref,
                    head_sha = excluded.head_sha,
                    updated_at = excluded.updated_at,
                    last_seen_at = excluded.last_seen_at
                "#,
                params![
                    row.pr_url,
                    row.owner,
                    row.repo,
                    row.number,
                    row.state,
                    row.title,
                    row.head_ref,
                    row.base_ref,
                    row.head_sha,
                    row.updated_at,
                    now,
                ],
            )?;
            Ok(())
        })
    }

    pub fn get_pr(&self, pr_url: &str) -> anyhow::Result<Option<StoredPr>> {
        self.with_conn(|conn| {
            conn.query_row(
                r#"
                SELECT
                    pr_url, owner, repo, number, state, title, head_ref, base_ref, head_sha,
                    updated_at, last_reviewed_sha, last_reviewed_updated_at
                FROM prs
                WHERE pr_url = ?1
                "#,
                [pr_url],
                |row| {
                    Ok(StoredPr {
                        pr_url: row.get(0)?,
                        owner: row.get(1)?,
                        repo: row.get(2)?,
                        number: row.get(3)?,
                        state: row.get(4)?,
                        title: row.get(5)?,
                        head_ref: row.get(6)?,
                        base_ref: row.get(7)?,
                        head_sha: row.get(8)?,
                        updated_at: row.get(9)?,
                        last_reviewed_sha: row.get(10)?,
                        last_reviewed_updated_at: row.get(11)?,
                    })
                },
            )
            .optional()
            .map_err(anyhow::Error::from)
        })
    }

    pub fn set_pr_review_marker(
        &self,
        pr_url: &str,
        reviewed_sha: &str,
        reviewed_updated_at: &str,
    ) -> anyhow::Result<()> {
        self.with_conn(|conn| {
            conn.execute(
                r#"
                UPDATE prs
                SET last_reviewed_sha = ?2, last_reviewed_updated_at = ?3
                WHERE pr_url = ?1
                "#,
                params![pr_url, reviewed_sha, reviewed_updated_at],
            )?;
            Ok(())
        })
    }

    pub fn insert_review(&self, row: &NewReview) -> anyhow::Result<()> {
        let now = unix_ts();
        self.with_conn(|conn| {
            conn.execute(
                r#"
                INSERT INTO reviews (
                    pr_url, provider, model, requires_code_changes, content_md, created_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                "#,
                params![
                    row.pr_url,
                    row.provider,
                    row.model,
                    bool_to_int(row.requires_code_changes),
                    row.content_md,
                    now,
                ],
            )?;
            Ok(())
        })
    }

    pub fn insert_fix_run(
        &self,
        pr_url: &str,
        provider: &str,
        status: &str,
        output: &str,
    ) -> anyhow::Result<()> {
        let now = unix_ts();
        self.with_conn(|conn| {
            conn.execute(
                r#"
                INSERT INTO fix_runs (pr_url, provider, status, output, created_at)
                VALUES (?1, ?2, ?3, ?4, ?5)
                "#,
                params![pr_url, provider, status, output, now],
            )?;
            Ok(())
        })
    }

    pub fn insert_sync_event(
        &self,
        pr_url: &str,
        status: &str,
        message: &str,
    ) -> anyhow::Result<()> {
        let now = unix_ts();
        self.with_conn(|conn| {
            conn.execute(
                r#"
                INSERT INTO sync_events (pr_url, status, message, created_at)
                VALUES (?1, ?2, ?3, ?4)
                "#,
                params![pr_url, status, message, now],
            )?;
            Ok(())
        })
    }

    pub fn mark_thread_done_local(&self, github_thread_id: &str) -> anyhow::Result<()> {
        self.with_conn(|conn| {
            conn.execute(
                "UPDATE threads SET done = 1, unread = 0 WHERE github_thread_id = ?1",
                [github_thread_id],
            )?;
            Ok(())
        })
    }

    pub fn latest_review_for_pr(
        &self,
        owner: &str,
        repo: &str,
        number: i64,
    ) -> anyhow::Result<Option<StoredReview>> {
        let pr_url = format!("https://github.com/{owner}/{repo}/pull/{number}");
        self.latest_review_by_url(&pr_url)
    }

    pub fn latest_review_by_url(&self, pr_url: &str) -> anyhow::Result<Option<StoredReview>> {
        self.with_conn(|conn| {
            conn.query_row(
                r#"
                SELECT id, pr_url, provider, model, requires_code_changes, content_md, created_at
                FROM reviews
                WHERE pr_url = ?1
                ORDER BY id DESC
                LIMIT 1
                "#,
                [pr_url],
                |row| {
                    let requires_code_changes: i64 = row.get(4)?;
                    Ok(StoredReview {
                        id: row.get(0)?,
                        pr_url: row.get(1)?,
                        provider: row.get(2)?,
                        model: row.get(3)?,
                        requires_code_changes: requires_code_changes != 0,
                        content_md: row.get(5)?,
                        created_at: row.get(6)?,
                    })
                },
            )
            .optional()
            .map_err(anyhow::Error::from)
        })
    }

    pub fn list_dashboard_threads(&self) -> anyhow::Result<Vec<DashboardThread>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                r#"
                SELECT
                    t.thread_key,
                    t.github_thread_id,
                    t.source,
                    t.repository,
                    t.subject_type,
                    t.subject_title,
                    t.subject_url,
                    t.reason,
                    t.pr_url,
                    t.unread,
                    t.done,
                    t.updated_at,
                    (
                        SELECT r.requires_code_changes
                        FROM reviews r
                        WHERE r.pr_url = t.pr_url
                        ORDER BY r.id DESC
                        LIMIT 1
                    ) AS latest_requires_code_changes,
                    p.state
                FROM threads t
                LEFT JOIN prs p ON p.pr_url = t.pr_url
                WHERE t.done = 0
                ORDER BY t.updated_at DESC
                "#,
            )?;

            let rows = stmt.query_map([], |row| {
                let unread: i64 = row.get(9)?;
                let done: i64 = row.get(10)?;
                let latest_requires: Option<i64> = row.get(12)?;
                Ok(DashboardThread {
                    thread_key: row.get(0)?,
                    github_thread_id: row.get(1)?,
                    source: row.get(2)?,
                    repository: row.get(3)?,
                    subject_type: row.get(4)?,
                    subject_title: row.get(5)?,
                    subject_url: row.get(6)?,
                    reason: row.get(7)?,
                    pr_url: row.get(8)?,
                    unread: unread != 0,
                    done: done != 0,
                    updated_at: row.get(11)?,
                    latest_requires_code_changes: latest_requires.map(|v| v != 0),
                    pr_state: row.get(13)?,
                })
            })?;

            let mut out = Vec::new();
            for row in rows {
                out.push(row?);
            }
            Ok(deduplicate_dashboard_threads(out))
        })
    }
}

fn deduplicate_dashboard_threads(threads: Vec<DashboardThread>) -> Vec<DashboardThread> {
    let mut deduped = Vec::new();
    let mut pr_indexes = HashMap::new();

    for thread in threads {
        let Some(pr_url) = thread.pr_url.clone() else {
            deduped.push(thread);
            continue;
        };

        if let Some(index) = pr_indexes.get(&pr_url).copied() {
            merge_dashboard_thread(&mut deduped[index], thread);
        } else {
            pr_indexes.insert(pr_url, deduped.len());
            deduped.push(thread);
        }
    }

    deduped
}

fn merge_dashboard_thread(existing: &mut DashboardThread, incoming: DashboardThread) {
    let existing_snapshot = existing.clone();
    let incoming_preferred =
        dashboard_thread_priority(&incoming) > dashboard_thread_priority(existing);

    if incoming_preferred {
        *existing = incoming.clone();
    }

    existing.source = merge_sources(&existing_snapshot.source, &incoming.source);
    existing.unread = existing_snapshot.unread || incoming.unread;
    existing.done = existing_snapshot.done && incoming.done;
    existing.updated_at = max_string(
        existing_snapshot.updated_at.clone(),
        incoming.updated_at.clone(),
    );
    existing.latest_requires_code_changes = existing_snapshot
        .latest_requires_code_changes
        .or(incoming.latest_requires_code_changes);
    existing.pr_state = existing_snapshot.pr_state.or(incoming.pr_state);
    existing.reason = merge_optional_string(
        incoming_preferred,
        existing_snapshot.reason,
        incoming.reason,
    );
    existing.subject_url = merge_optional_string(
        incoming_preferred,
        existing_snapshot.subject_url,
        incoming.subject_url,
    );
    existing.subject_type = merge_optional_string(
        incoming_preferred,
        existing_snapshot.subject_type,
        incoming.subject_type,
    );
}

fn dashboard_thread_priority(thread: &DashboardThread) -> usize {
    match (thread.github_thread_id.is_some(), thread.source.as_str()) {
        (true, _) => 2,
        (false, "my_pr") => 1,
        _ => 0,
    }
}

fn merge_sources(left: &str, right: &str) -> String {
    if left == right {
        return left.to_string();
    }

    let mut sources = Vec::new();
    for source in [left, right] {
        if !sources.iter().any(|existing| existing == &source) {
            sources.push(source);
        }
    }

    sources.sort_by_key(|source| match *source {
        "notification" => 0,
        "my_pr" => 1,
        _ => 2,
    });

    sources.join(" + ")
}

fn merge_optional_string(
    incoming_preferred: bool,
    existing: Option<String>,
    incoming: Option<String>,
) -> Option<String> {
    if incoming_preferred {
        incoming.or(existing)
    } else {
        existing.or(incoming)
    }
}

fn max_string(left: String, right: String) -> String {
    if right > left { right } else { left }
}

fn run_migrations(conn: &Connection) -> anyhow::Result<()> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS threads (
            thread_key TEXT PRIMARY KEY,
            github_thread_id TEXT,
            source TEXT NOT NULL,
            repository TEXT NOT NULL,
            subject_type TEXT,
            subject_title TEXT NOT NULL,
            subject_url TEXT,
            reason TEXT,
            pr_url TEXT,
            unread INTEGER NOT NULL,
            done INTEGER NOT NULL DEFAULT 0,
            updated_at TEXT NOT NULL,
            last_seen_at INTEGER NOT NULL
        );

        CREATE UNIQUE INDEX IF NOT EXISTS idx_threads_github_thread_id
            ON threads(github_thread_id)
            WHERE github_thread_id IS NOT NULL;

        CREATE INDEX IF NOT EXISTS idx_threads_pr_url ON threads(pr_url);

        CREATE TABLE IF NOT EXISTS prs (
            pr_url TEXT PRIMARY KEY,
            owner TEXT NOT NULL,
            repo TEXT NOT NULL,
            number INTEGER NOT NULL,
            state TEXT NOT NULL,
            title TEXT NOT NULL,
            head_ref TEXT NOT NULL,
            base_ref TEXT NOT NULL,
            head_sha TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            last_seen_at INTEGER NOT NULL,
            last_reviewed_sha TEXT,
            last_reviewed_updated_at TEXT
        );

        CREATE UNIQUE INDEX IF NOT EXISTS idx_prs_owner_repo_number
            ON prs(owner, repo, number);

        CREATE TABLE IF NOT EXISTS reviews (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            pr_url TEXT NOT NULL,
            provider TEXT NOT NULL,
            model TEXT,
            requires_code_changes INTEGER NOT NULL,
            content_md TEXT NOT NULL,
            created_at INTEGER NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_reviews_pr_url ON reviews(pr_url);

        CREATE TABLE IF NOT EXISTS fix_runs (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            pr_url TEXT NOT NULL,
            provider TEXT NOT NULL,
            status TEXT NOT NULL,
            output TEXT NOT NULL,
            created_at INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS sync_events (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            pr_url TEXT NOT NULL,
            status TEXT NOT NULL,
            message TEXT NOT NULL,
            created_at INTEGER NOT NULL
        );
        "#,
    )?;

    Ok(())
}

fn bool_to_int(value: bool) -> i64 {
    if value { 1 } else { 0 }
}

fn unix_ts() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
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
        path.push(format!("gigi-test-{ts}.sqlite"));
        Db::new(path).unwrap()
    }

    #[test]
    fn upsert_thread_is_idempotent() {
        let db = test_db();
        let row = NewThread {
            thread_key: "thread-1".to_string(),
            github_thread_id: Some("1".to_string()),
            source: "notification".to_string(),
            repository: "a/b".to_string(),
            subject_type: Some("PullRequest".to_string()),
            subject_title: "t".to_string(),
            subject_url: Some("u".to_string()),
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
    }

    #[test]
    fn done_threads_are_hidden_from_dashboard() {
        let db = test_db();
        let row = NewThread {
            thread_key: "thread-1".to_string(),
            github_thread_id: Some("1".to_string()),
            source: "notification".to_string(),
            repository: "a/b".to_string(),
            subject_type: Some("PullRequest".to_string()),
            subject_title: "t".to_string(),
            subject_url: Some("u".to_string()),
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
            thread_key: "thread-1".to_string(),
            github_thread_id: Some("1".to_string()),
            source: "notification".to_string(),
            repository: "a/b".to_string(),
            subject_type: Some("PullRequest".to_string()),
            subject_title: "t".to_string(),
            subject_url: Some("u".to_string()),
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
    fn dashboard_threads_deduplicate_notification_and_my_pr() {
        let db = test_db();
        let pr_url = "https://github.com/a/b/pull/1".to_string();

        db.upsert_pr(&NewPr {
            pr_url: pr_url.clone(),
            owner: "a".to_string(),
            repo: "b".to_string(),
            number: 1,
            state: "MERGED".to_string(),
            title: "Title".to_string(),
            head_ref: "feat".to_string(),
            base_ref: "main".to_string(),
            head_sha: "sha1".to_string(),
            updated_at: "2026-01-02T00:00:00Z".to_string(),
        })
        .unwrap();

        db.upsert_thread(&NewThread {
            thread_key: format!("mypr:{pr_url}"),
            github_thread_id: None,
            source: "my_pr".to_string(),
            repository: "a/b".to_string(),
            subject_type: Some("PullRequest".to_string()),
            subject_title: "Authored title".to_string(),
            subject_url: Some(pr_url.clone()),
            reason: Some("authored".to_string()),
            pr_url: Some(pr_url.clone()),
            unread: false,
            done: false,
            updated_at: "2026-01-02T00:00:00Z".to_string(),
        })
        .unwrap();

        db.upsert_thread(&NewThread {
            thread_key: "notif:123".to_string(),
            github_thread_id: Some("123".to_string()),
            source: "notification".to_string(),
            repository: "a/b".to_string(),
            subject_type: Some("PullRequest".to_string()),
            subject_title: "Notification title".to_string(),
            subject_url: Some(pr_url.clone()),
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
        assert_eq!(threads[0].source, "notification + my_pr");
        assert_eq!(threads[0].subject_title, "Notification title");
        assert_eq!(threads[0].updated_at, "2026-01-02T00:00:00Z");
        assert_eq!(threads[0].pr_state.as_deref(), Some("MERGED"));
        assert!(threads[0].unread);
    }
}
