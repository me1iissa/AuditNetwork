//! SQLite store. Two connection pools: a writer (single connection,
//! serialises writes through one connection in WAL mode) and a reader
//! (multi-connection, `query_only=1` for any user-facing SQL).

use std::path::Path;
use std::str::FromStr;

use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::{Pool, Sqlite};

const SCHEMA_SQL: &str = include_str!("../../../migrations/0001_init.sql");

#[derive(Clone)]
pub struct Store {
    pub writer: Pool<Sqlite>,
    pub reader: Pool<Sqlite>,
}

impl Store {
    pub async fn open(path: &Path) -> anyhow::Result<Self> {
        if let Some(dir) = path.parent() {
            if !dir.as_os_str().is_empty() {
                tokio::fs::create_dir_all(dir).await.ok();
            }
        }

        let url = format!("sqlite://{}", path.display());
        let base = SqliteConnectOptions::from_str(&url)?
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Normal)
            .busy_timeout(std::time::Duration::from_secs(10))
            .foreign_keys(true);

        let writer = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(base.clone())
            .await?;

        let reader = SqlitePoolOptions::new()
            .max_connections(8)
            .connect_with(base.read_only(false).pragma("query_only", "1"))
            .await?;

        let store = Self { writer, reader };
        store.run_migrations().await?;
        Ok(store)
    }

    async fn run_migrations(&self) -> anyhow::Result<()> {
        // Idempotent — every CREATE uses IF NOT EXISTS.
        sqlx::raw_sql(SCHEMA_SQL).execute(&self.writer).await?;
        // Pin schema version after migration.
        sqlx::query("PRAGMA user_version = 1").execute(&self.writer).await?;
        Ok(())
    }

    pub fn schema_sql() -> &'static str {
        SCHEMA_SQL
    }
}
