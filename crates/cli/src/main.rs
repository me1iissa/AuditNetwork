use std::path::PathBuf;

use anyhow::Context;
use clap::{Parser, Subcommand};
use store::Store;

#[derive(Parser, Debug)]
#[command(name = "auditnetwork", version, about = "Audit and visualise Claude Code tool-call activity")]
struct Cli {
    /// Path to the SQLite warehouse. Defaults to
    /// $XDG_DATA_HOME/auditnetwork/audit.db or ~/.local/share/auditnetwork/audit.db.
    #[arg(long, global = true)]
    db: Option<PathBuf>,

    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Ingest one or more Claude Code JSONL transcripts (or a directory of them).
    /// Pass --auto to default to ~/.claude/projects.
    Ingest {
        /// Path to a .jsonl file or a directory to scan recursively.
        path: Option<PathBuf>,
        /// Default to ~/.claude/projects.
        #[arg(long)]
        auto: bool,
    },
    /// Run a read-only SQL query against the warehouse and print rows as JSON.
    Query {
        sql: String,
    },
    /// Show top-level counts.
    Stats,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(env_filter).init();

    let cli = Cli::parse();
    let db_path = cli.db.unwrap_or_else(default_db_path);
    let store = Store::open(&db_path)
        .await
        .with_context(|| format!("open store at {}", db_path.display()))?;

    match cli.cmd {
        Cmd::Ingest { path, auto } => {
            let path = match (path, auto) {
                (Some(p), _) => p,
                (None, true) => default_claude_projects()
                    .ok_or_else(|| anyhow::anyhow!("could not resolve ~/.claude/projects"))?,
                (None, false) => {
                    anyhow::bail!("specify a path or pass --auto to ingest ~/.claude/projects");
                }
            };
            tracing::info!("ingesting {}", path.display());
            let stats = ingest::ingest_path(&store, &path).await?;
            println!(
                "files_seen={} files_ingested={} events_added={} tool_calls_added={} bytes_read={}",
                stats.files_seen,
                stats.files_ingested,
                stats.events_added,
                stats.tool_calls_added,
                stats.bytes_read,
            );
        }
        Cmd::Query { sql } => {
            run_query(&store, &sql).await?;
        }
        Cmd::Stats => {
            print_stats(&store).await?;
        }
    }
    Ok(())
}

fn default_db_path() -> PathBuf {
    let data = dirs::data_dir().unwrap_or_else(|| PathBuf::from("./.auditnetwork"));
    data.join("auditnetwork").join("audit.db")
}

fn default_claude_projects() -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    Some(home.join(".claude").join("projects"))
}

async fn run_query(store: &Store, sql: &str) -> anyhow::Result<()> {
    use sqlx::{Column, Row};
    let rows = sqlx::query(sql).fetch_all(&store.reader).await?;
    for row in rows {
        let cols = row.columns();
        let mut obj = serde_json::Map::new();
        for (i, col) in cols.iter().enumerate() {
            let name = col.name().to_string();
            let value: serde_json::Value = decode_cell(&row, i);
            obj.insert(name, value);
        }
        println!("{}", serde_json::Value::Object(obj));
    }
    Ok(())
}

fn decode_cell(row: &sqlx::sqlite::SqliteRow, idx: usize) -> serde_json::Value {
    use sqlx::Row;
    use sqlx::TypeInfo;
    use sqlx::ValueRef;
    let v = match row.try_get_raw(idx) {
        Ok(v) => v,
        Err(_) => return serde_json::Value::Null,
    };
    if v.is_null() {
        return serde_json::Value::Null;
    }
    let ty = v.type_info();
    match ty.name() {
        "INTEGER" | "INT" | "INT8" | "BIGINT" => row.try_get::<i64, _>(idx).map(serde_json::Value::from).unwrap_or(serde_json::Value::Null),
        "REAL" | "FLOAT" | "DOUBLE" => row.try_get::<f64, _>(idx).map(serde_json::Value::from).unwrap_or(serde_json::Value::Null),
        "BLOB" => row.try_get::<Vec<u8>, _>(idx).map(|b| serde_json::Value::String(hex::encode(b))).unwrap_or(serde_json::Value::Null),
        _ => row.try_get::<String, _>(idx).map(serde_json::Value::String).unwrap_or(serde_json::Value::Null),
    }
}

async fn print_stats(store: &Store) -> anyhow::Result<()> {
    for (label, q) in [
        ("sessions",        "SELECT COUNT(*) FROM sessions"),
        ("events",          "SELECT COUNT(*) FROM events"),
        ("messages",        "SELECT COUNT(*) FROM messages"),
        ("tool_calls",      "SELECT COUNT(*) FROM tool_calls"),
        ("tool_results",    "SELECT COUNT(*) FROM tool_results"),
        ("artifacts",       "SELECT COUNT(*) FROM artifacts"),
        ("edges",           "SELECT COUNT(*) FROM tool_artifact_edges"),
        ("file_ops",        "SELECT COUNT(*) FROM file_ops"),
        ("bash_commands",   "SELECT COUNT(*) FROM bash_commands"),
        ("web_fetches",     "SELECT COUNT(*) FROM web_fetches"),
        ("redactions",      "SELECT COUNT(*) FROM redactions"),
    ] {
        let n: i64 = sqlx::query_scalar(q).fetch_one(&store.reader).await?;
        println!("{label:>14}: {n}");
    }
    Ok(())
}

