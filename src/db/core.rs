use rusqlite::{OptionalExtension, params};

use crate::{
    github::Participant,
    review::{parse_requires_code_changes, sanitize_review_markdown},
};

use super::{
    Db, NewPr, NewReview, NewThread, StoredPr, StoredReview,
    util::{bool_to_int, normalize_review_storage, unix_ts},
};

impl Db {
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
                    subject_url, issue_state, discussion_answered, reason, pr_url, unread, done,
                    updated_at, is_draft, last_seen_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)
                ON CONFLICT(thread_key) DO UPDATE SET
                    github_thread_id = excluded.github_thread_id,
                    source = excluded.source,
                    repository = excluded.repository,
                    subject_type = excluded.subject_type,
                    subject_title = excluded.subject_title,
                    subject_url = excluded.subject_url,
                    issue_state = excluded.issue_state,
                    discussion_answered = excluded.discussion_answered,
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
                    row.discussion_answered.map(bool_to_int),
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
                    pr_url, owner, repo, number, state, merge_queue_state, title, head_ref,
                    base_ref, head_sha, updated_at, is_archived, is_draft, last_seen_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
                ON CONFLICT(pr_url) DO UPDATE SET
                    owner = excluded.owner,
                    repo = excluded.repo,
                    number = excluded.number,
                    state = excluded.state,
                    merge_queue_state = excluded.merge_queue_state,
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
                    row.merge_queue_state,
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
                    pr_url, owner, repo, number, state, merge_queue_state, title, head_ref,
                    base_ref, head_sha, updated_at, is_archived, last_reviewed_sha,
                    last_reviewed_updated_at
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
                        merge_queue_state: row.get(5)?,
                        title: row.get(6)?,
                        head_ref: row.get(7)?,
                        base_ref: row.get(8)?,
                        head_sha: row.get(9)?,
                        updated_at: row.get(10)?,
                        is_archived: row.get::<_, i64>(11)? != 0,
                        last_reviewed_sha: row.get(12)?,
                        last_reviewed_updated_at: row.get(13)?,
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

    pub fn mark_thread_read_local(&self, github_thread_id: &str) -> anyhow::Result<()> {
        self.with_conn(|conn| {
            conn.execute(
                "UPDATE threads SET unread = 0 WHERE github_thread_id = ?1",
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

    /// Replaces the stored participants for a PR with the given list.
    pub fn upsert_pr_participants(
        &self,
        pr_url: &str,
        participants: &[Participant],
    ) -> anyhow::Result<()> {
        self.with_conn(|conn| {
            conn.execute(
                "DELETE FROM pr_participants WHERE pr_url = ?1",
                params![pr_url],
            )?;
            let mut stmt = conn.prepare(
                "INSERT INTO pr_participants (pr_url, login, avatar_url, last_activity_at) VALUES (?1, ?2, ?3, ?4)",
            )?;
            for participant in participants {
                stmt.execute(params![
                    pr_url,
                    participant.login,
                    participant.avatar_url,
                    participant.last_activity_at
                ])?;
            }
            Ok(())
        })
    }

    /// Returns the stored participants for a PR, ordered by last activity (most recent first).
    pub fn get_pr_participants(&self, pr_url: &str) -> anyhow::Result<Vec<Participant>> {
        self.with_conn(|conn| {
            let mut stmt =
                conn.prepare("SELECT login, avatar_url, last_activity_at FROM pr_participants WHERE pr_url = ?1 ORDER BY last_activity_at DESC NULLS LAST")?;
            let rows = stmt.query_map(params![pr_url], |row| {
                Ok(Participant {
                    login: row.get(0)?,
                    avatar_url: row.get(1)?,
                    last_activity_at: row.get(2)?,
                })
            })?;
            let mut out = Vec::new();
            for row in rows {
                out.push(row?);
            }
            Ok(out)
        })
    }

    pub(super) fn sanitize_stored_reviews(&self) -> anyhow::Result<()> {
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
