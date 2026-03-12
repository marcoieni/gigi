use std::collections::HashMap;

use rusqlite::{OptionalExtension, params};

use super::{
    DashboardThread, DashboardThreadFilters, Db,
    util::{bool_to_int, unix_ts},
};

impl Db {
    #[cfg(test)]
    pub fn list_dashboard_threads(&self) -> anyhow::Result<Vec<DashboardThread>> {
        self.list_dashboard_threads_with_filters(&DashboardThreadFilters::default())
    }

    pub fn list_dashboard_threads_with_filters(
        &self,
        filters: &DashboardThreadFilters,
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
                    p.merge_queue_state,
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
                    pr_merge_queue_state: row.get(18)?,
                    is_archived_pr: row.get::<_, i64>(19)? != 0,
                    is_draft: row.get::<_, i64>(20)? != 0,
                    latest_review_content_md: row.get(21)?,
                    latest_review_created_at: row.get(22)?,
                    latest_review_provider: row.get(23)?,
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
                .filter(|thread| filters.include_repository(&thread.repository))
                .collect())
        })
    }

    pub fn dashboard_thread_filters(&self) -> anyhow::Result<DashboardThreadFilters> {
        self.with_conn(|conn| {
            let mut filters = conn
                .query_row(
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
                            selected_repositories: Vec::new(),
                        })
                    },
                )
                .optional()
                .map(|filters| filters.unwrap_or_default())?;

            let mut stmt =
                conn.prepare("SELECT repository FROM repository_filter ORDER BY repository")?;
            let repos = stmt.query_map([], |row| row.get::<_, String>(0))?;
            for repo in repos {
                filters.selected_repositories.push(repo?);
            }

            Ok(filters)
        })
    }

    pub fn set_dashboard_thread_filters(
        &self,
        filters: &DashboardThreadFilters,
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

    pub fn set_repository_filter(&self, repositories: &[String]) -> anyhow::Result<()> {
        self.with_conn(|conn| {
            conn.execute("DELETE FROM repository_filter", [])?;
            let mut stmt =
                conn.prepare("INSERT INTO repository_filter (repository) VALUES (?1)")?;
            for repo in repositories {
                stmt.execute(params![repo])?;
            }
            Ok(())
        })
    }

    pub fn list_all_repositories(&self) -> anyhow::Result<Vec<String>> {
        self.with_conn(|conn| {
            let mut stmt =
                conn.prepare("SELECT DISTINCT repository FROM threads ORDER BY repository")?;
            let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
            let mut out = Vec::new();
            for row in rows {
                out.push(row?);
            }
            Ok(out)
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
    pr_merge_queue_state: Option<String>,
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
            pr_merge_queue_state: self.pr_merge_queue_state,
            latest_review_content_md: self.latest_review_content_md,
            latest_review_created_at: self.latest_review_created_at,
            latest_review_provider: self.latest_review_provider,
            is_draft: self.is_draft,
            participants: Vec::new(),
        }
    }
}

impl DashboardThreadFilters {
    fn include_sources(&self, sources: &[String]) -> bool {
        sources.iter().all(|source| match source.as_str() {
            "notification" => self.show_notifications,
            "my_pr" => self.show_prs,
            _ => false,
        })
    }

    fn include_done_state(&self, done: bool) -> bool {
        (done && self.show_done) || (!done && self.show_not_done)
    }

    fn include_repository(&self, repository: &str) -> bool {
        self.selected_repositories.is_empty()
            || self
                .selected_repositories
                .iter()
                .any(|selected| selected == repository)
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
    existing.pr_merge_queue_state = existing_snapshot
        .pr_merge_queue_state
        .or(incoming.pr_merge_queue_state);
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
    if existing.participants.is_empty() {
        existing.participants = incoming.participants;
    }
}

fn dashboard_thread_priority(thread: &DashboardThread) -> usize {
    match (thread.github_thread_id.is_some(), thread.sources.as_slice()) {
        (true, _) => 2,
        (false, sources) if sources.iter().any(|source| source == "my_pr") => 1,
        _ => 0,
    }
}

fn merge_sources(left: &[String], right: &[String]) -> Vec<String> {
    let mut sources: Vec<String> = left.to_vec();
    for source in right {
        if !sources.contains(source) {
            sources.push(source.clone());
        }
    }
    sources.sort_by_key(|source| match source.as_str() {
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
