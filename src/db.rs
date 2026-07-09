use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub struct Database {
    path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub source_kind: String,
    pub source_url: String,
    pub play_url: String,
    pub title: Option<String>,
    pub uploader: Option<String>,
    pub duration: Option<i64>,
    pub thumbnail_url: Option<String>,
    pub created_at: String,
    pub last_played_at: Option<String>,
    pub play_count: i64,
}

impl Database {
    pub fn open(path: PathBuf) -> Result<Self> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create data directory {}", parent.display()))?;
        }

        let database = Self { path };
        database.init()?;
        Ok(database)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn record_play(
        &self,
        source_url: &str,
        play_url: &str,
        source: Option<&str>,
    ) -> Result<()> {
        let mut conn = self.connect()?;
        let now = now_text();
        let tx = conn.transaction()?;

        tx.execute(
            "INSERT OR IGNORE INTO tracks (
                source_kind,
                source_url,
                play_url,
                created_at
            ) VALUES (?1, ?2, ?3, ?4)",
            params!["url", source_url, play_url, now],
        )?;
        tx.execute(
            "UPDATE tracks
             SET play_count = play_count + 1,
                 last_played_at = ?1
             WHERE source_url = ?2",
            params![now, source_url],
        )?;

        let track_id: i64 = tx.query_row(
            "SELECT id FROM tracks WHERE source_url = ?1",
            params![source_url],
            |row| row.get(0),
        )?;
        tx.execute(
            "INSERT INTO plays (track_id, played_at, source)
             VALUES (?1, ?2, ?3)",
            params![track_id, now, source],
        )?;

        tx.commit()?;
        Ok(())
    }

    pub fn history(&self) -> Result<Vec<HistoryEntry>> {
        let conn = self.connect()?;
        let mut statement = conn.prepare(
            "SELECT
                source_kind,
                source_url,
                play_url,
                title,
                uploader,
                duration,
                thumbnail_url,
                created_at,
                last_played_at,
                play_count
             FROM tracks
             ORDER BY last_played_at DESC, created_at DESC",
        )?;

        let rows = statement.query_map([], |row| {
            Ok(HistoryEntry {
                source_kind: row.get(0)?,
                source_url: row.get(1)?,
                play_url: row.get(2)?,
                title: row.get(3)?,
                uploader: row.get(4)?,
                duration: row.get(5)?,
                thumbnail_url: row.get(6)?,
                created_at: row.get(7)?,
                last_played_at: row.get(8)?,
                play_count: row.get(9)?,
            })
        })?;

        rows.collect::<rusqlite::Result<Vec<_>>>()
            .with_context(|| "failed to read playback history")
    }

    fn init(&self) -> Result<()> {
        let conn = self.connect()?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS tracks (
                id INTEGER PRIMARY KEY,
                source_kind TEXT NOT NULL,
                source_url TEXT NOT NULL UNIQUE,
                play_url TEXT NOT NULL,
                title TEXT,
                uploader TEXT,
                duration INTEGER,
                thumbnail_url TEXT,
                created_at TEXT NOT NULL,
                last_played_at TEXT,
                play_count INTEGER NOT NULL DEFAULT 0
            );

            CREATE TABLE IF NOT EXISTS plays (
                id INTEGER PRIMARY KEY,
                track_id INTEGER NOT NULL,
                played_at TEXT NOT NULL,
                source TEXT,
                FOREIGN KEY(track_id) REFERENCES tracks(id)
            );",
        )
        .with_context(|| "failed to initialize playback history database")
    }

    fn connect(&self) -> Result<Connection> {
        Connection::open(&self.path)
            .with_context(|| format!("failed to open database {}", self.path.display()))
    }
}

fn now_text() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time before UNIX_EPOCH")
        .as_secs()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unique_db_path(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time before UNIX_EPOCH")
            .as_nanos();
        std::env::temp_dir().join(format!("ura-{name}-{nanos}.db"))
    }

    #[test]
    fn creates_database_and_records_history() {
        let path = unique_db_path("history");
        let database = Database::open(path.clone()).expect("open database");

        database
            .record_play(
                "https://youtu.be/example",
                "https://youtu.be/example",
                Some("cli"),
            )
            .expect("record play");

        let history = database.history().expect("read history");
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].source_url, "https://youtu.be/example");
        assert_eq!(history[0].play_url, "https://youtu.be/example");
        assert_eq!(history[0].play_count, 1);

        let _ = fs::remove_file(path);
    }

    #[test]
    fn repeated_plays_increment_play_count() {
        let path = unique_db_path("repeat");
        let database = Database::open(path.clone()).expect("open database");

        database
            .record_play(
                "https://youtu.be/example",
                "https://youtu.be/example",
                Some("cli"),
            )
            .expect("record first play");
        database
            .record_play(
                "https://youtu.be/example",
                "https://youtu.be/example",
                Some("cli"),
            )
            .expect("record second play");

        let history = database.history().expect("read history");
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].play_count, 2);

        let _ = fs::remove_file(path);
    }

    #[test]
    fn stores_source_and_play_urls_separately() {
        let path = unique_db_path("source-play");
        let database = Database::open(path.clone()).expect("open database");

        database
            .record_play(
                "https://www.youtube.com/watch?v=example&list=playlist",
                "https://www.youtube.com/watch?v=example",
                Some("cli"),
            )
            .expect("record play");

        let history = database.history().expect("read history");
        assert_eq!(history.len(), 1);
        assert_eq!(
            history[0].source_url,
            "https://www.youtube.com/watch?v=example&list=playlist"
        );
        assert_eq!(
            history[0].play_url,
            "https://www.youtube.com/watch?v=example"
        );

        let _ = fs::remove_file(path);
    }
}
