//! Database module — SQLite connection management and migrations.
//!
//! Uses `tokio-rusqlite` for async access to SQLite with WAL journal mode.

pub mod activity_repo;
pub mod agent_repo;
pub mod context_repo;
pub mod helpers;

use anyhow::{Context, Result};
use tokio_rusqlite::Connection;
use tracing::{info, warn};

/// The initial per-workspace migration SQL (embedded at compile time).
const MIGRATION_001: &str = include_str!("migrations/001_initial.sql");

/// Wraps a `tokio_rusqlite::Connection` and provides migration support.
#[derive(Clone)]
pub struct Database {
    conn: Connection,
}

impl Database {
    /// Opens (or creates) a SQLite database at the given path and runs
    /// all pending migrations.
    pub async fn open_and_migrate(path: &str) -> Result<Self> {
        let conn = Connection::open(path)
            .await
            .with_context(|| format!("Failed to open SQLite database at {path}"))?;

        let db = Self { conn };
        db.run_migrations().await?;
        Ok(db)
    }

    /// Opens an in-memory SQLite database — useful for testing.
    pub async fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()
            .await
            .context("Failed to open in-memory SQLite database")?;

        let db = Self { conn };
        db.run_migrations().await?;
        Ok(db)
    }

    /// Returns a reference to the underlying connection for repository use.
    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    /// Executes all migration scripts that haven't been applied yet.
    ///
    /// Uses a simple `schema_version` PRAGMA to track which migrations have run.
    async fn run_migrations(&self) -> Result<()> {
        self.conn
            .call(|conn| {
                // Enable WAL and foreign keys
                conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;

                // Check current schema version
                let version: i64 =
                    conn.pragma_query_value(None, "user_version", |row| row.get(0))?;

                if version < 1 {
                    info!("Running migration 001_initial.sql ...");
                    conn.execute_batch(MIGRATION_001)?;
                    conn.pragma_update(None, "user_version", 1)?;
                    info!("Migration 001 applied successfully. Schema version = 1");
                } else {
                    info!("Schema version = {version}, no migrations needed");
                }

                Ok(())
            })
            .await
            .map_err(|e| {
                warn!("Migration failed: {e}");
                anyhow::anyhow!("Migration failed: {e}")
            })?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_open_in_memory_and_migrate() {
        let db = Database::open_in_memory().await.unwrap();

        // Verify tables were created
        let tables: Vec<String> = db
            .conn()
            .call(|conn| {
                let mut stmt = conn
                    .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")?;
                let rows = stmt.query_map([], |row| row.get(0))?;
                let mut names = Vec::new();
                for row in rows {
                    names.push(row?);
                }
                Ok(names)
            })
            .await
            .unwrap();

        let expected_tables = [
            "agent_activities",
            "agents",
            "embedding_cache",
            "file_locks",
            "memory_entries",
            "shared_contexts",
            "skills",
            "tasks",
        ];

        for table in &expected_tables {
            assert!(
                tables.contains(&table.to_string()),
                "Expected table '{table}' not found. Got: {tables:?}"
            );
        }
    }

    #[tokio::test]
    async fn test_migration_is_idempotent() {
        let db = Database::open_in_memory().await.unwrap();

        // Running migrations again should not error
        db.run_migrations().await.unwrap();

        // Schema version should still be 1
        let version: i64 = db
            .conn()
            .call(|conn| {
                let v = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
                Ok(v)
            })
            .await
            .unwrap();

        assert_eq!(version, 1);
    }

    #[tokio::test]
    async fn test_foreign_keys_enabled() {
        let db = Database::open_in_memory().await.unwrap();

        let fk_enabled: i64 = db
            .conn()
            .call(|conn| {
                let v = conn.pragma_query_value(None, "foreign_keys", |row| row.get(0))?;
                Ok(v)
            })
            .await
            .unwrap();

        assert_eq!(fk_enabled, 1);
    }

    #[tokio::test]
    async fn test_indexes_created() {
        let db = Database::open_in_memory().await.unwrap();

        let indexes: Vec<String> = db
            .conn()
            .call(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT name FROM sqlite_master WHERE type='index' AND name LIKE 'idx_%' ORDER BY name",
                )?;
                let rows = stmt.query_map([], |row| row.get(0))?;
                let mut names = Vec::new();
                for row in rows {
                    names.push(row?);
                }
                Ok(names)
            })
            .await
            .unwrap();

        let expected_indexes = [
            "idx_activities_ws_time",
            "idx_agents_ws",
            "idx_file_locks_ws",
            "idx_memory_ws_type",
            "idx_tasks_ws_status",
        ];

        for idx in &expected_indexes {
            assert!(
                indexes.contains(&idx.to_string()),
                "Expected index '{idx}' not found. Got: {indexes:?}"
            );
        }
    }
}
