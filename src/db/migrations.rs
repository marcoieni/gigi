use rusqlite::Connection;

pub(super) fn run_migrations(conn: &Connection) -> anyhow::Result<()> {
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
            discussion_answered INTEGER,
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
        CREATE INDEX IF NOT EXISTS idx_threads_source_subject_url ON threads(source, subject_url);

        CREATE TABLE IF NOT EXISTS prs (
            pr_url TEXT PRIMARY KEY,
            owner TEXT NOT NULL,
            repo TEXT NOT NULL,
            number INTEGER NOT NULL,
            state TEXT NOT NULL,
            merge_queue_state TEXT,
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
            show_my_prs INTEGER NOT NULL DEFAULT 1,
            show_assigned_issues INTEGER NOT NULL DEFAULT 1,
            show_done INTEGER NOT NULL,
            show_not_done INTEGER NOT NULL,
            group_by_repository INTEGER NOT NULL DEFAULT 1,
            updated_at INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS kv (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS pr_participants (
            pr_url TEXT NOT NULL,
            login TEXT NOT NULL,
            avatar_url TEXT NOT NULL,
            PRIMARY KEY (pr_url, login)
        );

        CREATE INDEX IF NOT EXISTS idx_pr_participants_pr_url ON pr_participants(pr_url);
        "#,
    )?;

    add_column_if_missing(conn, "prs", "is_archived", "INTEGER NOT NULL DEFAULT 0")?;
    add_column_if_missing(conn, "prs", "is_draft", "INTEGER NOT NULL DEFAULT 0")?;
    add_column_if_missing(conn, "prs", "merge_queue_state", "TEXT")?;
    add_column_if_missing(conn, "threads", "issue_state", "TEXT")?;
    add_column_if_missing(conn, "threads", "discussion_answered", "INTEGER")?;
    add_column_if_missing(conn, "threads", "is_draft", "INTEGER NOT NULL DEFAULT 0")?;
    add_column_if_missing(
        conn,
        "dashboard_preferences",
        "group_by_repository",
        "INTEGER NOT NULL DEFAULT 1",
    )?;
    add_column_if_missing(
        conn,
        "dashboard_preferences",
        "show_my_prs",
        "INTEGER NOT NULL DEFAULT 1",
    )?;
    add_column_if_missing(
        conn,
        "dashboard_preferences",
        "show_assigned_issues",
        "INTEGER NOT NULL DEFAULT 1",
    )?;
    add_column_if_missing(conn, "pr_participants", "last_activity_at", "TEXT")?;

    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS repository_filter (
            repository TEXT PRIMARY KEY
        );
        "#,
    )?;

    Ok(())
}

fn add_column_if_missing(
    conn: &Connection,
    table: &str,
    column: &str,
    definition: &str,
) -> anyhow::Result<bool> {
    let pragma = format!("PRAGMA table_info({table})");
    let mut stmt = conn.prepare(&pragma)?;
    let columns = stmt.query_map([], |row| row.get::<_, String>(1))?;
    for existing in columns {
        if existing? == column {
            return Ok(false);
        }
    }

    let alter = format!("ALTER TABLE {table} ADD COLUMN {column} {definition}");
    conn.execute(&alter, [])?;
    Ok(true)
}
