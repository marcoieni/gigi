#[cfg(feature = "ssr")]
mod core;
#[cfg(feature = "ssr")]
mod dashboard;
#[cfg(feature = "ssr")]
mod migrations;
mod models;
#[cfg(feature = "ssr")]
mod util;

#[cfg(all(feature = "ssr", test))]
mod tests;

pub use models::{
    DashboardThread, DashboardThreadFilters, NewPr, NewReview, NewThread, StoredPr, StoredReview,
};

#[cfg(feature = "ssr")]
use std::path::{Path, PathBuf};

#[cfg(feature = "ssr")]
use anyhow::Context as _;
#[cfg(feature = "ssr")]
use rusqlite::Connection;

#[cfg(feature = "ssr")]
#[derive(Debug, Clone)]
pub struct Db {
    path: PathBuf,
}

#[cfg(feature = "ssr")]
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
