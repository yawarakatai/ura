use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::config::generate_token;
use crate::mpv::MediaMetadata;

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorizedClient {
    pub name: String,
    pub created_at: String,
    pub last_seen_at: Option<String>,
    pub revoked_at: Option<String>,
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

    pub fn update_track_metadata(
        &self,
        source_url: &str,
        metadata: &MediaMetadata,
    ) -> Result<bool> {
        let conn = self.connect()?;
        let title = clean_optional(metadata.title.as_deref());
        let uploader = clean_optional(metadata.uploader.as_deref().or(metadata.artist.as_deref()));
        let duration = metadata
            .duration_seconds
            .map(|duration| duration.round() as i64);
        let thumbnail_url = clean_optional(metadata.thumbnail_url.as_deref());

        let current = conn
            .query_row(
                "SELECT title, uploader, duration, thumbnail_url
                 FROM tracks
                 WHERE source_url = ?1",
                params![source_url],
                |row| {
                    Ok((
                        row.get::<_, Option<String>>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, Option<i64>>(2)?,
                        row.get::<_, Option<String>>(3)?,
                    ))
                },
            )
            .optional()
            .with_context(|| "failed to read current track metadata")?;

        let Some((current_title, current_uploader, current_duration, current_thumbnail_url)) =
            current
        else {
            return Ok(false);
        };

        let next_title = title.or(current_title.clone());
        let next_uploader = uploader.or(current_uploader.clone());
        let next_duration = duration.or(current_duration);
        let next_thumbnail_url = thumbnail_url.or(current_thumbnail_url.clone());

        if next_title == current_title
            && next_uploader == current_uploader
            && next_duration == current_duration
            && next_thumbnail_url == current_thumbnail_url
        {
            return Ok(false);
        }

        conn.execute(
            "UPDATE tracks
             SET title = ?1,
                 uploader = ?2,
                 duration = ?3,
                 thumbnail_url = ?4
             WHERE source_url = ?5",
            params![
                next_title,
                next_uploader,
                next_duration,
                next_thumbnail_url,
                source_url
            ],
        )
        .with_context(|| "failed to update track metadata")?;
        Ok(true)
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

    pub fn authorize_client(&self, name: &str, token: &str) -> Result<()> {
        validate_client_name(name)?;
        let token_hash = hash_token(token);
        let conn = self.connect()?;
        conn.execute(
            "INSERT INTO authorized_devices (name, token_hash, created_at)
             VALUES (?1, ?2, ?3)",
            params![name, token_hash, now_text()],
        )
        .with_context(|| format!("failed to authorize client `{name}`"))?;
        Ok(())
    }

    pub fn revoke_authorized_client(&self, name: &str) -> Result<bool> {
        validate_client_name(name)?;
        let conn = self.connect()?;
        let changed = conn
            .execute(
                "UPDATE authorized_devices
                 SET revoked_at = COALESCE(revoked_at, ?1)
                 WHERE name = ?2 AND revoked_at IS NULL",
                params![now_text(), name],
            )
            .with_context(|| format!("failed to revoke client `{name}`"))?;
        Ok(changed > 0)
    }

    pub fn authorized_clients(&self) -> Result<Vec<AuthorizedClient>> {
        let conn = self.connect()?;
        let mut statement = conn.prepare(
            "SELECT name, created_at, last_seen_at, revoked_at
             FROM authorized_devices
             WHERE revoked_at IS NULL
             ORDER BY name",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(AuthorizedClient {
                name: row.get(0)?,
                created_at: row.get(1)?,
                last_seen_at: row.get(2)?,
                revoked_at: row.get(3)?,
            })
        })?;

        rows.collect::<rusqlite::Result<Vec<_>>>()
            .with_context(|| "failed to read authorized clients")
    }

    pub fn authenticate_authorized_token(&self, token: &str) -> Result<bool> {
        let token_hash = hash_token(token);
        let conn = self.connect()?;
        let mut statement = conn.prepare(
            "SELECT id, token_hash, last_seen_at
             FROM authorized_devices
             WHERE revoked_at IS NULL",
        )?;
        let mut rows = statement.query([])?;
        while let Some(row) = rows.next()? {
            let id: i64 = row.get(0)?;
            let stored_hash: String = row.get(1)?;
            let last_seen_at: Option<String> = row.get(2)?;
            if constant_time_eq(token_hash.as_bytes(), stored_hash.as_bytes()) {
                update_last_seen_if_stale(&conn, id, last_seen_at.as_deref())?;
                return Ok(true);
            }
        }
        Ok(false)
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
            );

            CREATE TABLE IF NOT EXISTS authorized_devices (
                id INTEGER PRIMARY KEY,
                name TEXT NOT NULL,
                token_hash TEXT NOT NULL,
                created_at TEXT NOT NULL,
                last_seen_at TEXT,
                revoked_at TEXT
            );

            CREATE UNIQUE INDEX IF NOT EXISTS authorized_devices_active_name_idx
                ON authorized_devices(name)
                WHERE revoked_at IS NULL;",
        )
        .with_context(|| "failed to initialize playback history database")
    }

    fn connect(&self) -> Result<Connection> {
        Connection::open(&self.path)
            .with_context(|| format!("failed to open database {}", self.path.display()))
    }
}

pub fn hash_token(token: &str) -> String {
    hex_encode(&Sha256::digest(token.as_bytes()))
}

pub fn validate_client_name(name: &str) -> Result<()> {
    if name.trim().is_empty() {
        anyhow::bail!("client name must not be empty");
    }
    Ok(())
}

pub fn authorize_client(database: &Database, name: &str) -> Result<String> {
    let token = generate_token()?;
    database.authorize_client(name, &token)?;
    Ok(token)
}

fn update_last_seen_if_stale(conn: &Connection, id: i64, last_seen_at: Option<&str>) -> Result<()> {
    let now = now_text();
    let should_update = last_seen_at
        .and_then(|value| value.parse::<u64>().ok())
        .and_then(|last_seen| {
            now.parse::<u64>()
                .ok()
                .map(|now| now.saturating_sub(last_seen))
        })
        .map(|age| age >= 60)
        .unwrap_or(true);

    if should_update {
        conn.execute(
            "UPDATE authorized_devices SET last_seen_at = ?1 WHERE id = ?2",
            params![now, id],
        )
        .with_context(|| "failed to update authorized client last_seen_at")?;
    }
    Ok(())
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0_u8, |diff, (left, right)| diff | (left ^ right))
        == 0
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

fn now_text() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time before UNIX_EPOCH")
        .as_secs()
        .to_string()
}

fn clean_optional(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
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

    #[test]
    fn metadata_update_enriches_existing_track() {
        let path = unique_db_path("metadata");
        let database = Database::open(path.clone()).expect("open database");

        database
            .record_play(
                "https://youtu.be/example",
                "https://youtu.be/example",
                Some("cli"),
            )
            .expect("record play");
        let changed = database
            .update_track_metadata(
                "https://youtu.be/example",
                &MediaMetadata {
                    title: Some("Example song".to_string()),
                    uploader: Some("Example artist".to_string()),
                    duration_seconds: Some(222.5),
                    thumbnail_url: Some("https://i.ytimg.com/vi/example/hqdefault.jpg".to_string()),
                    ..MediaMetadata::default()
                },
            )
            .expect("update metadata");
        assert!(changed);

        let history = database.history().expect("read history");
        assert_eq!(history[0].title.as_deref(), Some("Example song"));
        assert_eq!(history[0].uploader.as_deref(), Some("Example artist"));
        assert_eq!(history[0].duration, Some(223));
        assert_eq!(
            history[0].thumbnail_url.as_deref(),
            Some("https://i.ytimg.com/vi/example/hqdefault.jpg")
        );

        let _ = fs::remove_file(path);
    }

    #[test]
    fn empty_metadata_does_not_overwrite_useful_metadata() {
        let path = unique_db_path("metadata-preserve");
        let database = Database::open(path.clone()).expect("open database");

        database
            .record_play(
                "https://youtu.be/example",
                "https://youtu.be/example",
                Some("cli"),
            )
            .expect("record play");
        let changed = database
            .update_track_metadata(
                "https://youtu.be/example",
                &MediaMetadata {
                    title: Some("Useful title".to_string()),
                    duration_seconds: Some(42.0),
                    ..MediaMetadata::default()
                },
            )
            .expect("set metadata");
        assert!(changed);
        let changed = database
            .update_track_metadata(
                "https://youtu.be/example",
                &MediaMetadata {
                    title: Some("   ".to_string()),
                    duration_seconds: None,
                    ..MediaMetadata::default()
                },
            )
            .expect("empty metadata update");
        assert!(!changed);

        let history = database.history().expect("read history");
        assert_eq!(history[0].title.as_deref(), Some("Useful title"));
        assert_eq!(history[0].duration, Some(42));

        let _ = fs::remove_file(path);
    }

    #[test]
    fn identical_metadata_update_reports_no_change() {
        let path = unique_db_path("metadata-identical");
        let database = Database::open(path.clone()).expect("open database");

        database
            .record_play(
                "https://youtu.be/example",
                "https://youtu.be/example",
                Some("cli"),
            )
            .expect("record play");
        let metadata = MediaMetadata {
            title: Some("Example song".to_string()),
            uploader: Some("Example artist".to_string()),
            duration_seconds: Some(222.5),
            thumbnail_url: Some("https://i.ytimg.com/vi/example/hqdefault.jpg".to_string()),
            ..MediaMetadata::default()
        };

        assert!(
            database
                .update_track_metadata("https://youtu.be/example", &metadata)
                .expect("first update")
        );
        assert!(
            !database
                .update_track_metadata("https://youtu.be/example", &metadata)
                .expect("second update")
        );

        let _ = fs::remove_file(path);
    }

    #[test]
    fn authorized_device_tokens_authenticate_and_revoke() {
        let path = unique_db_path("authorized-device");
        let database = Database::open(path.clone()).expect("open database");

        database
            .authorize_client("desuwa", "0123456789abcdef0123456789abcdef")
            .expect("authorize device");

        assert!(
            database
                .authenticate_authorized_token("0123456789abcdef0123456789abcdef")
                .expect("authenticate")
        );
        assert!(
            !database
                .authenticate_authorized_token("wrong-wrong-wrong-wrong-wrong-wrong")
                .expect("authenticate wrong")
        );
        assert!(database.revoke_authorized_client("desuwa").expect("revoke"));
        assert!(
            !database
                .authenticate_authorized_token("0123456789abcdef0123456789abcdef")
                .expect("authenticate revoked")
        );

        let _ = fs::remove_file(path);
    }

    #[test]
    fn authorized_devices_store_hashes_and_last_seen() {
        let path = unique_db_path("authorized-device-hash");
        let database = Database::open(path.clone()).expect("open database");
        let token = "abcdef0123456789abcdef0123456789";

        database
            .authorize_client("firefox", token)
            .expect("authorize device");
        assert!(
            database
                .authenticate_authorized_token(token)
                .expect("authenticate")
        );

        let conn = Connection::open(&path).expect("open raw connection");
        let (token_hash, last_seen_at): (String, Option<String>) = conn
            .query_row(
                "SELECT token_hash, last_seen_at FROM authorized_devices WHERE name = 'firefox'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("read authorized row");
        assert_ne!(token_hash, token);
        assert_eq!(token_hash, hash_token(token));
        assert!(last_seen_at.is_some());

        let _ = fs::remove_file(path);
    }

    #[test]
    fn failed_authentication_does_not_update_last_seen() {
        let path = unique_db_path("authorized-device-failed");
        let database = Database::open(path.clone()).expect("open database");

        database
            .authorize_client("firefox", "abcdef0123456789abcdef0123456789")
            .expect("authorize device");
        assert!(
            !database
                .authenticate_authorized_token("wrong-wrong-wrong-wrong-wrong-wrong")
                .expect("authenticate wrong")
        );

        let conn = Connection::open(&path).expect("open raw connection");
        let last_seen_at: Option<String> = conn
            .query_row(
                "SELECT last_seen_at FROM authorized_devices WHERE name = 'firefox'",
                [],
                |row| row.get(0),
            )
            .expect("read authorized row");
        assert_eq!(last_seen_at, None);

        let _ = fs::remove_file(path);
    }

    #[test]
    fn authorized_device_migration_preserves_history() {
        let path = unique_db_path("authorized-device-migration");
        let database = Database::open(path.clone()).expect("open database");
        database
            .record_play(
                "https://youtu.be/example",
                "https://youtu.be/example",
                Some("cli"),
            )
            .expect("record play");
        drop(database);

        let database = Database::open(path.clone()).expect("reopen database");
        database
            .authorize_client("desuwa", "0123456789abcdef0123456789abcdef")
            .expect("authorize");

        let history = database.history().expect("history");
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].source_url, "https://youtu.be/example");

        let _ = fs::remove_file(path);
    }
}
