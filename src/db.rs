use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::Context as _;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::review::{parse_requires_code_changes, sanitize_review_markdown};

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
    pub issue_state: Option<String>,
    pub reason: Option<String>,
    pub pr_url: Option<String>,
    pub unread: bool,
    pub done: bool,
    pub updated_at: String,
    pub is_draft: bool,
}

/// Data required to insert a new PR row. Does not include DB-managed fields.
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
    pub is_archived: bool,
    pub is_draft: bool,
}

/// PR row as read from the DB. Extends [`NewPr`] with DB-managed fields
/// (`last_reviewed_sha`, `last_reviewed_updated_at`).
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
    pub is_archived: bool,
    pub last_reviewed_sha: Option<String>,
    pub last_reviewed_updated_at: Option<String>,
}

/// Review row as read from the DB. Extends [`NewReview`] with DB-managed fields
/// (`id`, `created_at`).
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

/// Data required to insert a new review row. Does not include DB-managed fields.
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
    pub sources: Vec<String>,
    pub repository: String,
    pub pr_owner: Option<String>,
    pub pr_repo: Option<String>,
    pub pr_number: Option<i64>,
    pub subject_type: Option<String>,
    pub subject_title: String,
    pub subject_url: Option<String>,
    pub issue_state: Option<String>,
    pub reason: Option<String>,
    pub pr_url: Option<String>,
    pub unread: bool,
    pub done: bool,
    pub updated_at: String,
    pub latest_requires_code_changes: Option<bool>,
    pub pr_state: Option<String>,
    pub latest_review_content_md: Option<String>,
    pub latest_review_created_at: Option<i64>,
    pub latest_review_provider: Option<String>,
    pub is_draft: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct DashboardThreadFilters {
    pub show_notifications: bool,
    pub show_prs: bool,
    pub show_done: bool,
    pub show_not_done: bool,
    pub group_by_repository: bool,
}

impl Default for DashboardThreadFilters {
    fn default() -> Self {
        Self {
            show_notifications: true,
            show_prs: true,
            show_done: false,
            show_not_done: true,
            group_by_repository: true,
        }
    }
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
        db.sanitize_stored_reviews()?;

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

    pub fn get_kv(&self, key: &str) -> anyhow::Result<Option<String>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare("SELECT value FROM kv WHERE key = ?1")?;
            let result = stmt.query_row([key], |row| row.get(0)).optional()?;
            Ok(result)
        })
    }

    pub fn set_kv(&self, key: &str, value: &str) -> anyhow::Result<()> {
        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO kv (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                [key, value],
            )?;
            Ok(())
        })
    }

    pub fn upsert_thread(&self, row: &NewThread) -> anyhow::Result<()> {
        let now = unix_ts();
        self.with_conn(|conn| {
            conn.execute(
                r#"
                INSERT INTO threads (
                    thread_key, github_thread_id, source, repository, subject_type, subject_title,
                    subject_url, issue_state, reason, pr_url, unread, done, updated_at, is_draft, last_seen_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
                ON CONFLICT(thread_key) DO UPDATE SET
                    github_thread_id = excluded.github_thread_id,
                    source = excluded.source,
                    repository = excluded.repository,
                    subject_type = excluded.subject_type,
                    subject_title = excluded.subject_title,
                    subject_url = excluded.subject_url,
                    issue_state = excluded.issue_state,
                    reason = excluded.reason,
                    pr_url = excluded.pr_url,
                    unread = excluded.unread,
                    done = MAX(threads.done, excluded.done),
                    updated_at = excluded.updated_at,
                    is_draft = excluded.is_draft,
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
                    row.issue_state,
                    row.reason,
                    row.pr_url,
                    bool_to_int(row.unread),
                    bool_to_int(row.done),
                    row.updated_at,
                    bool_to_int(row.is_draft),
                    now,
                ],
            )?;
            Ok(())
        })
    }

    pub fn delete_threads_by_source_and_pr_urls(
        &self,
        source: &str,
        pr_urls: &[String],
    ) -> anyhow::Result<()> {
        if pr_urls.is_empty() {
            return Ok(());
        }
        self.with_conn(|conn| {
            let mut sql = String::from("DELETE FROM threads WHERE source = ?1 AND pr_url IN (");
            for idx in 0..pr_urls.len() {
                if idx > 0 {
                    sql.push_str(", ");
                }
                sql.push('?');
                sql.push_str(&(idx + 2).to_string());
            }
            sql.push(')');

            let mut stmt = conn.prepare(&sql)?;
            let params = std::iter::once(source.to_string())
                .chain(pr_urls.iter().cloned())
                .collect::<Vec<_>>();
            stmt.execute(rusqlite::params_from_iter(params))?;
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
                    updated_at, is_archived, is_draft, last_seen_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
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
                    is_archived = excluded.is_archived,
                    is_draft = excluded.is_draft,
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
                    bool_to_int(row.is_archived),
                    bool_to_int(row.is_draft),
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
                    updated_at, is_archived, last_reviewed_sha, last_reviewed_updated_at
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
                        is_archived: row.get::<_, i64>(10)? != 0,
                        last_reviewed_sha: row.get(11)?,
                        last_reviewed_updated_at: row.get(12)?,
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
        let (content_md, requires_code_changes) =
            normalize_review_storage(&row.content_md, row.requires_code_changes);
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
                    bool_to_int(requires_code_changes),
                    content_md,
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

    pub fn mark_authored_pr_done_local(&self, pr_url: &str) -> anyhow::Result<()> {
        self.with_conn(|conn| {
            conn.execute(
                "UPDATE threads SET done = 1, unread = 0 WHERE source = 'my_pr' AND pr_url = ?1",
                [pr_url],
            )?;
            Ok(())
        })
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
                    let stored_requires_code_changes: i64 = row.get(4)?;
                    let content_md = sanitize_review_markdown(&row.get::<_, String>(5)?);
                    Ok(StoredReview {
                        id: row.get(0)?,
                        pr_url: row.get(1)?,
                        provider: row.get(2)?,
                        model: row.get(3)?,
                        requires_code_changes: parse_requires_code_changes(&content_md)
                            .unwrap_or(stored_requires_code_changes != 0),
                        content_md,
                        created_at: row.get(6)?,
                    })
                },
            )
            .optional()
            .map_err(anyhow::Error::from)
        })
    }

    #[cfg(test)]
    pub fn list_dashboard_threads(&self) -> anyhow::Result<Vec<DashboardThread>> {
        self.list_dashboard_threads_with_filters(DashboardThreadFilters::default())
    }

    pub fn list_dashboard_threads_with_filters(
        &self,
        filters: DashboardThreadFilters,
    ) -> anyhow::Result<Vec<DashboardThread>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                r#"
                SELECT
                    t.thread_key,
                    t.github_thread_id,
                    t.source,
                    t.repository,
                    p.owner,
                    p.repo,
                    p.number,
                    t.subject_type,
                    t.subject_title,
                    t.subject_url,
                    t.issue_state,
                    t.reason,
                    t.pr_url,
                    t.unread,
                    t.done,
                    t.updated_at,
                    lr.requires_code_changes AS latest_requires_code_changes,
                    p.state,
                    COALESCE(p.is_archived, 0),
                    MAX(COALESCE(t.is_draft, 0), COALESCE(p.is_draft, 0)) AS is_draft,
                    lr.content_md AS latest_review_content_md,
                    lr.created_at AS latest_review_created_at,
                    lr.provider AS latest_review_provider
                FROM threads t
                LEFT JOIN prs p ON p.pr_url = t.pr_url
                LEFT JOIN (
                    SELECT
                        r.pr_url,
                        r.requires_code_changes,
                        r.content_md,
                        r.created_at,
                        r.provider
                    FROM reviews r
                    INNER JOIN (
                        SELECT pr_url, MAX(id) AS max_id
                        FROM reviews
                        GROUP BY pr_url
                    ) latest ON latest.pr_url = r.pr_url AND latest.max_id = r.id
                ) lr ON lr.pr_url = t.pr_url
                ORDER BY t.updated_at DESC
                "#,
            )?;

            let rows = stmt.query_map([], |row| {
                let unread: i64 = row.get(13)?;
                let done: i64 = row.get(14)?;
                let latest_requires: Option<i64> = row.get(16)?;
                Ok(DashboardThreadRow {
                    thread_key: row.get(0)?,
                    github_thread_id: row.get(1)?,
                    source: row.get(2)?,
                    repository: row.get(3)?,
                    pr_owner: row.get(4)?,
                    pr_repo: row.get(5)?,
                    pr_number: row.get(6)?,
                    subject_type: row.get(7)?,
                    subject_title: row.get(8)?,
                    subject_url: row.get(9)?,
                    issue_state: row.get(10)?,
                    reason: row.get(11)?,
                    pr_url: row.get(12)?,
                    unread: unread != 0,
                    done: done != 0,
                    updated_at: row.get(15)?,
                    latest_requires_code_changes: latest_requires.map(|v| v != 0),
                    pr_state: row.get(17)?,
                    is_archived_pr: row.get::<_, i64>(18)? != 0,
                    is_draft: row.get::<_, i64>(19)? != 0,
                    latest_review_content_md: row.get(20)?,
                    latest_review_created_at: row.get(21)?,
                    latest_review_provider: row.get(22)?,
                })
            })?;

            let mut out = Vec::new();
            for row in rows {
                let row = row?;
                if row.is_archived_pr && row.pr_state.as_deref() == Some("OPEN") {
                    continue;
                }
                if !filters.include_sources(std::slice::from_ref(&row.source)) {
                    continue;
                }
                out.push(row.into_dashboard_thread());
            }
            let deduped = deduplicate_dashboard_threads(out);
            Ok(deduped
                .into_iter()
                .filter(|thread| filters.include_done_state(thread.done))
                .collect())
        })
    }

    pub fn dashboard_thread_filters(&self) -> anyhow::Result<DashboardThreadFilters> {
        self.with_conn(|conn| {
            conn.query_row(
                r#"
                SELECT
                    show_notifications,
                    show_prs,
                    show_done,
                    show_not_done,
                    group_by_repository
                FROM dashboard_preferences
                WHERE id = 1
                "#,
                [],
                |row| {
                    Ok(DashboardThreadFilters {
                        show_notifications: row.get::<_, i64>(0)? != 0,
                        show_prs: row.get::<_, i64>(1)? != 0,
                        show_done: row.get::<_, i64>(2)? != 0,
                        show_not_done: row.get::<_, i64>(3)? != 0,
                        group_by_repository: row.get::<_, i64>(4)? != 0,
                    })
                },
            )
            .optional()
            .map(|filters| filters.unwrap_or_default())
            .map_err(anyhow::Error::from)
        })
    }

    pub fn set_dashboard_thread_filters(
        &self,
        filters: DashboardThreadFilters,
    ) -> anyhow::Result<()> {
        let now = unix_ts();
        self.with_conn(|conn| {
            conn.execute(
                r#"
                INSERT INTO dashboard_preferences (
                    id,
                    show_notifications,
                    show_prs,
                    show_done,
                    show_not_done,
                    group_by_repository,
                    updated_at
                ) VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6)
                ON CONFLICT(id) DO UPDATE SET
                    show_notifications = excluded.show_notifications,
                    show_prs = excluded.show_prs,
                    show_done = excluded.show_done,
                    show_not_done = excluded.show_not_done,
                    group_by_repository = excluded.group_by_repository,
                    updated_at = excluded.updated_at
                "#,
                params![
                    bool_to_int(filters.show_notifications),
                    bool_to_int(filters.show_prs),
                    bool_to_int(filters.show_done),
                    bool_to_int(filters.show_not_done),
                    bool_to_int(filters.group_by_repository),
                    now,
                ],
            )?;
            Ok(())
        })
    }

    fn sanitize_stored_reviews(&self) -> anyhow::Result<()> {
        self.with_conn(|conn| {
            let mut stmt =
                conn.prepare("SELECT id, requires_code_changes, content_md FROM reviews")?;
            let rows = stmt.query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)? != 0,
                    row.get::<_, String>(2)?,
                ))
            })?;

            let mut updates = Vec::new();
            for row in rows {
                let (id, stored_requires_code_changes, content_md) = row?;
                let (sanitized, requires_code_changes) =
                    normalize_review_storage(&content_md, stored_requires_code_changes);
                if sanitized != content_md || requires_code_changes != stored_requires_code_changes
                {
                    updates.push((id, requires_code_changes, sanitized));
                }
            }
            drop(stmt);

            for (id, requires_code_changes, sanitized) in updates {
                conn.execute(
                    "UPDATE reviews SET requires_code_changes = ?2, content_md = ?3 WHERE id = ?1",
                    params![id, bool_to_int(requires_code_changes), sanitized],
                )?;
            }

            Ok(())
        })
    }
}

fn normalize_review_storage(
    content_md: &str,
    stored_requires_code_changes: bool,
) -> (String, bool) {
    let sanitized = sanitize_review_markdown(content_md);
    let requires_code_changes =
        parse_requires_code_changes(&sanitized).unwrap_or(stored_requires_code_changes);
    (sanitized, requires_code_changes)
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

#[derive(Debug, Clone)]
struct DashboardThreadRow {
    thread_key: String,
    github_thread_id: Option<String>,
    source: String,
    repository: String,
    pr_owner: Option<String>,
    pr_repo: Option<String>,
    pr_number: Option<i64>,
    subject_type: Option<String>,
    subject_title: String,
    subject_url: Option<String>,
    issue_state: Option<String>,
    reason: Option<String>,
    pr_url: Option<String>,
    unread: bool,
    done: bool,
    updated_at: String,
    latest_requires_code_changes: Option<bool>,
    pr_state: Option<String>,
    is_archived_pr: bool,
    latest_review_content_md: Option<String>,
    latest_review_created_at: Option<i64>,
    latest_review_provider: Option<String>,
    is_draft: bool,
}

impl DashboardThreadRow {
    fn into_dashboard_thread(self) -> DashboardThread {
        DashboardThread {
            thread_key: self.thread_key,
            github_thread_id: self.github_thread_id,
            sources: vec![self.source],
            repository: self.repository,
            pr_owner: self.pr_owner,
            pr_repo: self.pr_repo,
            pr_number: self.pr_number,
            subject_type: self.subject_type,
            subject_title: self.subject_title,
            subject_url: self.subject_url,
            issue_state: self.issue_state,
            reason: self.reason,
            pr_url: self.pr_url,
            unread: self.unread,
            done: self.done,
            updated_at: self.updated_at,
            latest_requires_code_changes: self.latest_requires_code_changes,
            pr_state: self.pr_state,
            latest_review_content_md: self.latest_review_content_md,
            latest_review_created_at: self.latest_review_created_at,
            latest_review_provider: self.latest_review_provider,
            is_draft: self.is_draft,
        }
    }
}

impl DashboardThreadFilters {
    fn include_sources(self, sources: &[String]) -> bool {
        sources.iter().all(|s| match s.as_str() {
            "notification" => self.show_notifications,
            "my_pr" => self.show_prs,
            _ => false,
        })
    }

    fn include_done_state(self, done: bool) -> bool {
        (done && self.show_done) || (!done && self.show_not_done)
    }
}

fn merge_dashboard_thread(existing: &mut DashboardThread, incoming: DashboardThread) {
    let existing_snapshot = existing.clone();
    let incoming_preferred =
        dashboard_thread_priority(&incoming) > dashboard_thread_priority(existing);

    if incoming_preferred {
        *existing = incoming.clone();
    }

    existing.sources = merge_sources(&existing_snapshot.sources, &incoming.sources);
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
    existing.pr_owner = existing_snapshot.pr_owner.or(incoming.pr_owner);
    existing.pr_repo = existing_snapshot.pr_repo.or(incoming.pr_repo);
    existing.pr_number = existing_snapshot.pr_number.or(incoming.pr_number);
    existing.issue_state = existing_snapshot.issue_state.or(incoming.issue_state);
    existing.latest_review_content_md = existing_snapshot
        .latest_review_content_md
        .or(incoming.latest_review_content_md);
    existing.latest_review_created_at = existing_snapshot
        .latest_review_created_at
        .or(incoming.latest_review_created_at);
    existing.latest_review_provider = existing_snapshot
        .latest_review_provider
        .or(incoming.latest_review_provider);
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
    existing.is_draft = existing_snapshot.is_draft || incoming.is_draft;
}

fn dashboard_thread_priority(thread: &DashboardThread) -> usize {
    match (thread.github_thread_id.is_some(), thread.sources.as_slice()) {
        (true, _) => 2,
        (false, sources) if sources.iter().any(|s| s == "my_pr") => 1,
        _ => 0,
    }
}

fn merge_sources(left: &[String], right: &[String]) -> Vec<String> {
    let mut sources: Vec<String> = left.to_vec();
    for s in right {
        if !sources.contains(s) {
            sources.push(s.clone());
        }
    }
    sources.sort_by_key(|s| match s.as_str() {
        "notification" => 0,
        "my_pr" => 1,
        _ => 2,
    });
    sources
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
    // Enable WAL mode so the axum handlers and the poll task can access the DB concurrently.
    conn.execute_batch("PRAGMA journal_mode=WAL;")?;
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
            issue_state TEXT,
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
            is_archived INTEGER NOT NULL DEFAULT 0,
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

        CREATE TABLE IF NOT EXISTS dashboard_preferences (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            show_notifications INTEGER NOT NULL,
            show_prs INTEGER NOT NULL,
            show_done INTEGER NOT NULL,
            show_not_done INTEGER NOT NULL,
            group_by_repository INTEGER NOT NULL DEFAULT 1,
            updated_at INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS kv (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );
        "#,
    )?;

    add_column_if_missing(conn, "prs", "is_archived", "INTEGER NOT NULL DEFAULT 0")?;
    add_column_if_missing(conn, "prs", "is_draft", "INTEGER NOT NULL DEFAULT 0")?;
    add_column_if_missing(conn, "threads", "issue_state", "TEXT")?;
    add_column_if_missing(conn, "threads", "is_draft", "INTEGER NOT NULL DEFAULT 0")?;
    add_column_if_missing(
        conn,
        "dashboard_preferences",
        "group_by_repository",
        "INTEGER NOT NULL DEFAULT 1",
    )?;

    Ok(())
}

fn add_column_if_missing(
    conn: &Connection,
    table: &str,
    column: &str,
    definition: &str,
) -> anyhow::Result<()> {
    let pragma = format!("PRAGMA table_info({table})");
    let mut stmt = conn.prepare(&pragma)?;
    let columns = stmt.query_map([], |row| row.get::<_, String>(1))?;
    for existing in columns {
        if existing? == column {
            return Ok(());
        }
    }

    let alter = format!("ALTER TABLE {table} ADD COLUMN {column} {definition}");
    conn.execute(&alter, [])?;
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
            .list_dashboard_threads_with_filters(DashboardThreadFilters {
                show_notifications: true,
                show_prs: true,
                show_done: true,
                show_not_done: false,
                group_by_repository: true,
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
            .list_dashboard_threads_with_filters(DashboardThreadFilters {
                show_notifications: false,
                show_prs: true,
                show_done: false,
                show_not_done: true,
                group_by_repository: true,
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
        };

        db.set_dashboard_thread_filters(filters).unwrap();

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
    fn dashboard_threads_expose_issue_state() {
        let db = test_db();

        db.upsert_thread(&NewThread {
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
}
