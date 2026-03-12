mod core;
mod dashboard;
mod migrations;
mod models;
mod util;

#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};

use anyhow::Context as _;
use rusqlite::Connection;

pub use models::{
    DashboardThread, DashboardThreadFilters, NewPr, NewReview, NewThread, StoredPr, StoredReview,
};

#[derive(Debug, Clone)]
pub struct Db {
    path: PathBuf,
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
            migrations::run_migrations(conn)?;
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
}
