use std::{
    collections::{HashMap, VecDeque},
    io::{BufRead, BufReader, Write},
    os::unix::net::UnixStream,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tracing::{debug, warn};

use crate::db::Database;
use crate::error::UraError;

pub struct MpvClient {
    stream: UnixStream,
    next_request_id: u64,
}

static NEXT_COMMAND_REQUEST_ID: AtomicU64 = AtomicU64::new(1);

pub const OBSERVE_MEDIA_TITLE: i64 = 1;
pub const OBSERVE_DURATION: i64 = 2;
pub const OBSERVE_METADATA: i64 = 3;
pub const OBSERVE_PATH: i64 = 4;
pub const OBSERVE_PAUSE: i64 = 5;
pub const OBSERVE_IDLE_ACTIVE: i64 = 6;
pub const OBSERVE_PLAYLIST_POS: i64 = 7;
pub const OBSERVE_PLAYLIST_COUNT: i64 = 8;
pub const OBSERVE_TIME_POS: i64 = 9;

const OBSERVED_PROPERTIES: [(i64, &str); 9] = [
    (OBSERVE_MEDIA_TITLE, "media-title"),
    (OBSERVE_DURATION, "duration"),
    (OBSERVE_METADATA, "metadata"),
    (OBSERVE_PATH, "path"),
    (OBSERVE_PAUSE, "pause"),
    (OBSERVE_IDLE_ACTIVE, "idle-active"),
    (OBSERVE_PLAYLIST_POS, "playlist-pos"),
    (OBSERVE_PLAYLIST_COUNT, "playlist-count"),
    (OBSERVE_TIME_POS, "time-pos"),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoopMode {
    Off,
    One,
    Queue,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LoopStatus {
    Off,
    One,
    Queue,
    Custom {
        loop_file: Option<String>,
        loop_playlist: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MpvStatus {
    pub pause: Option<bool>,
    pub idle_active: Option<bool>,
    pub path: Option<String>,
    pub media_title: Option<String>,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub uploader: Option<String>,
    pub album: Option<String>,
    pub duration_seconds: Option<f64>,
    pub position_seconds: Option<f64>,
    pub playlist_pos: Option<i64>,
    pub playlist_count: Option<i64>,
    pub source_url: Option<String>,
    pub playback_path: Option<String>,
    pub loop_status: Option<LoopStatus>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MediaMetadata {
    pub source_url: Option<String>,
    pub playback_path: Option<String>,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub uploader: Option<String>,
    pub album: Option<String>,
    pub duration_seconds: Option<f64>,
    pub thumbnail_url: Option<String>,
}

#[derive(Debug, Clone)]
struct PendingPlayback {
    source_url: String,
    play_url: String,
    source: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct ObservedPlaybackState {
    pending: VecDeque<PendingPlayback>,
    pub current_source_url: Option<String>,
    pub current_play_url: Option<String>,
    pub current_source: Option<String>,
    pub pause: Option<bool>,
    pub idle_active: Option<bool>,
    pub path: Option<String>,
    pub media_title: Option<String>,
    pub duration_seconds: Option<f64>,
    pub position_seconds: Option<f64>,
    pub playlist_pos: Option<i64>,
    pub playlist_count: Option<i64>,
    pub metadata: HashMap<String, String>,
    pub normalized_metadata: MediaMetadata,
}

pub type SharedPlaybackState = Arc<Mutex<ObservedPlaybackState>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueMode {
    Replace,
    Append,
}

#[derive(Debug, Deserialize)]
struct MpvResponse {
    error: String,
    data: Option<Value>,
    request_id: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct MpvEventMessage {
    event: Option<String>,
    id: Option<i64>,
    name: Option<String>,
    data: Option<Value>,
}

#[derive(Debug, Clone, Default)]
pub struct MetadataSnapshot {
    media_title: Option<String>,
    duration_seconds: Option<f64>,
    metadata: HashMap<String, String>,
    path: Option<String>,
    pause: Option<bool>,
    idle_active: Option<bool>,
    playlist_pos: Option<i64>,
    playlist_count: Option<i64>,
    position_seconds: Option<f64>,
}

pub struct MpvEventObserver {
    shutdown: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl MpvClient {
    pub fn connect(socket_path: &Path) -> Result<Self> {
        let stream = UnixStream::connect(socket_path)?;
        Ok(Self {
            stream,
            next_request_id: NEXT_COMMAND_REQUEST_ID.fetch_add(1000, Ordering::Relaxed),
        })
    }

    pub fn load_replace(&mut self, url: &str) -> Result<()> {
        self.send_command(loadfile_command(url, LoadMode::Replace))?;
        self.set_pause(false)
    }

    pub fn load_enqueue(&mut self, url: &str) -> Result<()> {
        self.send_command(loadfile_command(url, LoadMode::AppendPlay))?;
        Ok(())
    }

    pub fn pause(&mut self) -> Result<()> {
        self.set_pause(true)
    }

    pub fn resume(&mut self) -> Result<()> {
        self.set_pause(false)
    }

    pub fn toggle(&mut self) -> Result<()> {
        self.send_command(json!({ "command": ["cycle", "pause"] }))?;
        Ok(())
    }

    pub fn stop(&mut self) -> Result<()> {
        self.send_command(json!({ "command": ["stop"] }))?;
        Ok(())
    }

    pub fn set_loop_mode(&mut self, mode: LoopMode) -> Result<()> {
        for command in loop_mode_commands(mode) {
            self.send_command(command)?;
        }
        Ok(())
    }

    pub fn loop_status(&mut self) -> Result<LoopStatus> {
        Ok(normalize_loop_status(
            self.get_loop_property("loop-file")?,
            self.get_loop_property("loop-playlist")?,
        ))
    }

    pub fn status(&mut self) -> Result<MpvStatus> {
        Ok(MpvStatus {
            pause: self.get_bool_property("pause")?,
            idle_active: self.get_bool_property("idle-active")?,
            path: self.get_string_property("path")?,
            media_title: self.get_string_property("media-title")?,
            title: None,
            artist: None,
            uploader: None,
            album: None,
            duration_seconds: self.get_number_property("duration")?,
            position_seconds: self.get_number_property("time-pos")?,
            playlist_pos: self.get_i64_property("playlist-pos")?,
            playlist_count: self.get_i64_property("playlist-count")?,
            source_url: None,
            playback_path: None,
            loop_status: Some(self.loop_status()?),
        })
    }

    pub fn metadata_snapshot(&mut self) -> Result<MetadataSnapshot> {
        Ok(MetadataSnapshot {
            media_title: self.get_string_property("media-title")?,
            duration_seconds: self.get_number_property("duration")?,
            metadata: self.get_metadata_property()?,
            path: self.get_string_property("path")?,
            pause: self.get_bool_property("pause")?,
            idle_active: self.get_bool_property("idle-active")?,
            playlist_pos: self.get_i64_property("playlist-pos")?,
            playlist_count: self.get_i64_property("playlist-count")?,
            position_seconds: self.get_number_property("time-pos")?,
        })
    }

    pub fn control(&mut self, command: &str) -> Result<()> {
        match command {
            "toggle" => self.toggle(),
            "stop" => self.stop(),
            "pause" => self.pause(),
            "resume" => self.resume(),
            other => Err(UraError::UnsupportedControlCommand(other.to_string()).into()),
        }
    }

    fn set_pause(&mut self, pause: bool) -> Result<()> {
        self.send_command(json!({ "command": ["set_property", "pause", pause] }))?;
        Ok(())
    }

    fn get_bool_property(&mut self, name: &str) -> Result<Option<bool>> {
        let response = match self.send_command(json!({ "command": ["get_property", name] })) {
            Ok(response) => response,
            Err(error) if is_property_unavailable(&error) => return Ok(None),
            Err(error) => return Err(error),
        };
        match response.data {
            Some(Value::Bool(value)) => Ok(Some(value)),
            Some(Value::Null) | None => Ok(None),
            Some(value) => bail!(UraError::InvalidMpvResponse(value.to_string())),
        }
    }

    fn get_string_property(&mut self, name: &str) -> Result<Option<String>> {
        let response = match self.send_command(json!({ "command": ["get_property", name] })) {
            Ok(response) => response,
            Err(error) if is_property_unavailable(&error) => return Ok(None),
            Err(error) => return Err(error),
        };
        match response.data {
            Some(Value::String(value)) => Ok(Some(value)),
            Some(Value::Null) | None => Ok(None),
            Some(value) => bail!(UraError::InvalidMpvResponse(value.to_string())),
        }
    }

    fn get_loop_property(&mut self, name: &str) -> Result<Option<String>> {
        let response = match self.send_command(json!({ "command": ["get_property", name] })) {
            Ok(response) => response,
            Err(error) if is_property_unavailable(&error) => return Ok(None),
            Err(error) => return Err(error),
        };
        match response.data {
            Some(Value::String(value)) => Ok(Some(value)),
            Some(Value::Bool(false)) => Ok(Some("no".to_string())),
            Some(Value::Bool(true)) => Ok(Some("yes".to_string())),
            Some(Value::Null) | None => Ok(None),
            Some(value) => bail!(UraError::InvalidMpvResponse(value.to_string())),
        }
    }

    fn get_number_property(&mut self, name: &str) -> Result<Option<f64>> {
        let response = match self.send_command(json!({ "command": ["get_property", name] })) {
            Ok(response) => response,
            Err(error) if is_property_unavailable(&error) => return Ok(None),
            Err(error) => return Err(error),
        };
        match response.data {
            Some(Value::Number(value)) => Ok(value.as_f64()),
            Some(Value::Null) | None => Ok(None),
            Some(value) => bail!(UraError::InvalidMpvResponse(value.to_string())),
        }
    }

    fn get_i64_property(&mut self, name: &str) -> Result<Option<i64>> {
        let response = match self.send_command(json!({ "command": ["get_property", name] })) {
            Ok(response) => response,
            Err(error) if is_property_unavailable(&error) => return Ok(None),
            Err(error) => return Err(error),
        };
        match response.data {
            Some(Value::Number(value)) => Ok(value.as_i64()),
            Some(Value::Null) | None => Ok(None),
            Some(value) => bail!(UraError::InvalidMpvResponse(value.to_string())),
        }
    }

    fn get_metadata_property(&mut self) -> Result<HashMap<String, String>> {
        let response = match self.send_command(json!({ "command": ["get_property", "metadata"] })) {
            Ok(response) => response,
            Err(error) if is_property_unavailable(&error) => return Ok(HashMap::new()),
            Err(error) => return Err(error),
        };
        match response.data {
            Some(Value::Object(map)) => Ok(metadata_from_value_map(map)),
            Some(Value::Null) | None => Ok(HashMap::new()),
            Some(value) => bail!(UraError::InvalidMpvResponse(value.to_string())),
        }
    }

    fn send_command(&mut self, command: Value) -> Result<MpvResponse> {
        let request_id = self.next_request_id;
        self.next_request_id += 1;
        let command = with_request_id(command, request_id);
        let mut line = serde_json::to_vec(&command)?;
        line.push(b'\n');
        self.stream.write_all(&line)?;
        self.stream.flush()?;

        let mut reader = BufReader::new(self.stream.try_clone()?);
        loop {
            let mut response = String::new();
            let bytes = reader.read_line(&mut response)?;
            if bytes == 0 {
                bail!("mpv IPC closed before command response");
            }
            if is_event_message(&response) {
                continue;
            }
            let parsed = parse_response(&response)?;
            if parsed.request_id == Some(request_id) || parsed.request_id.is_none() {
                return Ok(parsed);
            }
        }
    }
}

impl MpvEventObserver {
    pub fn start(
        socket_path: PathBuf,
        database: Arc<Database>,
        state: SharedPlaybackState,
    ) -> Self {
        let shutdown = Arc::new(AtomicBool::new(false));
        let thread_shutdown = Arc::clone(&shutdown);
        let handle = thread::spawn(move || {
            run_event_observer(socket_path, database, state, thread_shutdown);
        });

        Self {
            shutdown,
            handle: Some(handle),
        }
    }
}

impl Drop for MpvEventObserver {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

pub fn shared_playback_state() -> SharedPlaybackState {
    Arc::new(Mutex::new(ObservedPlaybackState::default()))
}

pub fn register_playback_request(
    state: &SharedPlaybackState,
    source_url: String,
    play_url: String,
    source: Option<String>,
    mode: QueueMode,
) {
    let mut state = state.lock().expect("playback state lock poisoned");
    if mode == QueueMode::Replace {
        state.pending.clear();
        state.current_source_url = None;
        state.current_play_url = None;
        state.current_source = None;
        state.normalized_metadata = MediaMetadata::default();
    }
    state.pending.push_back(PendingPlayback {
        source_url,
        play_url,
        source,
    });
}

pub fn rollback_playback_request(state: &SharedPlaybackState, play_url: &str) {
    let mut state = state.lock().expect("playback state lock poisoned");
    if let Some(index) = state
        .pending
        .iter()
        .rposition(|pending| pending.play_url == play_url)
    {
        state.pending.remove(index);
    }
}

pub fn observed_status(state: &SharedPlaybackState, loop_status: Option<LoopStatus>) -> MpvStatus {
    let state = state.lock().expect("playback state lock poisoned");
    MpvStatus {
        pause: state.pause,
        idle_active: state.idle_active,
        path: state.path.clone(),
        media_title: state.media_title.clone(),
        title: state.normalized_metadata.title.clone(),
        artist: state.normalized_metadata.artist.clone(),
        uploader: state.normalized_metadata.uploader.clone(),
        album: state.normalized_metadata.album.clone(),
        duration_seconds: state.duration_seconds,
        position_seconds: state.position_seconds,
        playlist_pos: state.playlist_pos,
        playlist_count: state.playlist_count,
        source_url: state.current_source_url.clone(),
        playback_path: state.current_play_url.clone(),
        loop_status,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LoadMode {
    Replace,
    AppendPlay,
}

impl LoadMode {
    fn as_mpv_arg(self) -> &'static str {
        match self {
            Self::Replace => "replace",
            Self::AppendPlay => "append-play",
        }
    }
}

fn loadfile_command(url: &str, mode: LoadMode) -> Value {
    json!({ "command": ["loadfile", url, mode.as_mpv_arg()] })
}

fn set_property_command(name: &str, value: &str) -> Value {
    json!({ "command": ["set_property", name, value] })
}

fn loop_mode_properties(mode: LoopMode) -> [(&'static str, &'static str); 2] {
    match mode {
        LoopMode::Off => [("loop-file", "no"), ("loop-playlist", "no")],
        LoopMode::One => [("loop-file", "inf"), ("loop-playlist", "no")],
        LoopMode::Queue => [("loop-file", "no"), ("loop-playlist", "inf")],
    }
}

fn loop_mode_commands(mode: LoopMode) -> [Value; 2] {
    loop_mode_properties(mode).map(|(name, value)| set_property_command(name, value))
}

fn with_request_id(mut command: Value, request_id: u64) -> Value {
    if let Value::Object(ref mut map) = command {
        map.insert("request_id".to_string(), Value::from(request_id));
    }
    command
}

fn is_event_message(response: &str) -> bool {
    serde_json::from_str::<Value>(response)
        .ok()
        .and_then(|value| value.get("event").cloned())
        .is_some()
}

fn run_event_observer(
    socket_path: PathBuf,
    database: Arc<Database>,
    state: SharedPlaybackState,
    shutdown: Arc<AtomicBool>,
) {
    while !shutdown.load(Ordering::Relaxed) {
        match UnixStream::connect(&socket_path) {
            Ok(mut stream) => {
                if let Err(error) = stream.set_read_timeout(Some(Duration::from_millis(200))) {
                    warn!(error = %error, "failed to set mpv IPC observer timeout");
                }
                match observe_events(&mut stream, &socket_path, &database, &state, &shutdown) {
                    Ok(()) => return,
                    Err(error) if shutdown.load(Ordering::Relaxed) => {
                        debug!(error = %error, "mpv IPC observer stopped")
                    }
                    Err(error) => warn!(error = %error, "mpv IPC observer disconnected"),
                }
            }
            Err(error) if shutdown.load(Ordering::Relaxed) => {
                debug!(error = %error, "mpv IPC observer stopped before connecting");
                return;
            }
            Err(_) => thread::sleep(Duration::from_millis(100)),
        }
    }
}

fn observe_events(
    stream: &mut UnixStream,
    socket_path: &Path,
    database: &Database,
    state: &SharedPlaybackState,
    shutdown: &AtomicBool,
) -> Result<()> {
    for (request_id, (id, name)) in (1..).zip(OBSERVED_PROPERTIES) {
        let command = json!({ "command": ["observe_property", id, name] });
        let command = with_request_id(command, request_id);
        let mut line = serde_json::to_vec(&command)?;
        line.push(b'\n');
        stream.write_all(&line)?;
    }
    stream.flush()?;
    debug!("mpv IPC observer registered properties");

    // A file can finish loading before the observer has registered, or while it
    // is reconnecting. Reconcile the snapshot so its pending request is not
    // permanently omitted from history.
    match MpvClient::connect(socket_path).and_then(|mut client| client.metadata_snapshot()) {
        Ok(snapshot) if snapshot.idle_active != Some(true) && snapshot.path.is_some() => {
            handle_file_loaded(snapshot, database, state)?;
        }
        Ok(_) => {}
        Err(error) => warn!(error = %error, "failed to reconcile mpv state"),
    }

    let mut reader = BufReader::new(stream.try_clone()?);
    while !shutdown.load(Ordering::Relaxed) {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) => bail!("mpv IPC observer connection closed"),
            Ok(_) => {
                if let Err(error) = handle_observer_line(&line, socket_path, database, state) {
                    warn!(error = %error, "failed to process mpv IPC event");
                }
            }
            Err(error)
                if error.kind() == std::io::ErrorKind::WouldBlock
                    || error.kind() == std::io::ErrorKind::TimedOut => {}
            Err(error) => return Err(error.into()),
        }
    }

    Ok(())
}

fn handle_observer_line(
    line: &str,
    socket_path: &Path,
    database: &Database,
    state: &SharedPlaybackState,
) -> Result<()> {
    let value: Value = serde_json::from_str(line)
        .map_err(|_| UraError::InvalidMpvResponse(line.trim().to_string()))?;
    if value.get("request_id").is_some() && value.get("event").is_none() {
        return Ok(());
    }
    let event: MpvEventMessage = serde_json::from_value(value)?;
    match event.event.as_deref() {
        Some("property-change") => {
            if handle_property_change(event, state) {
                update_current_track_metadata(database, state)?;
            }
            Ok(())
        }
        Some("file-loaded") => {
            let snapshot = MpvClient::connect(socket_path)
                .and_then(|mut client| client.metadata_snapshot())
                .unwrap_or_else(|error| {
                    warn!(error = %error, "metadata_missing");
                    MetadataSnapshot::default()
                });
            handle_file_loaded(snapshot, database, state)
        }
        Some("end-file") => {
            handle_end_file(state);
            Ok(())
        }
        Some("shutdown") => bail!("mpv shutdown event received"),
        Some(_) | None => Ok(()),
    }
}

fn handle_file_loaded(
    snapshot: MetadataSnapshot,
    database: &Database,
    state: &SharedPlaybackState,
) -> Result<()> {
    let (pending, source_url, metadata) = {
        let mut state = state.lock().expect("playback state lock poisoned");
        apply_snapshot(&mut state, snapshot);
        let pending = take_matching_pending(&mut state);
        if let Some(pending) = &pending {
            state.current_source_url = Some(pending.source_url.clone());
            state.current_play_url = Some(pending.play_url.clone());
            state.current_source = pending.source.clone();
        }
        let source_url = state.current_source_url.clone();
        refresh_normalized_metadata(&mut state);
        let metadata = state.normalized_metadata.clone();
        (pending, source_url, metadata)
    };

    let Some(source_url) = source_url else {
        debug!("metadata_missing");
        return Ok(());
    };

    if let Some(pending) = pending {
        if let Err(error) = database.record_play(
            &pending.source_url,
            &pending.play_url,
            pending.source.as_deref(),
        ) {
            state
                .lock()
                .expect("playback state lock poisoned")
                .pending
                .push_front(pending);
            return Err(error);
        }
        debug!("media_loaded");
    }
    database.update_track_metadata(&source_url, &metadata)?;
    Ok(())
}

fn update_current_track_metadata(database: &Database, state: &SharedPlaybackState) -> Result<()> {
    let (source_url, metadata) = {
        let state = state.lock().expect("playback state lock poisoned");
        (
            state.current_source_url.clone(),
            state.normalized_metadata.clone(),
        )
    };
    if let Some(source_url) = source_url
        && database.update_track_metadata(&source_url, &metadata)?
    {
        debug!("metadata_updated");
    }
    Ok(())
}

fn handle_property_change(event: MpvEventMessage, state: &SharedPlaybackState) -> bool {
    let mut state = state.lock().expect("playback state lock poisoned");
    let metadata_changed = match event.name.as_deref() {
        Some("media-title") | None if event.id == Some(OBSERVE_MEDIA_TITLE) => {
            state.media_title = clean_string_value(event.data);
            true
        }
        Some("duration") | None if event.id == Some(OBSERVE_DURATION) => {
            state.duration_seconds = number_value(event.data);
            true
        }
        Some("metadata") | None if event.id == Some(OBSERVE_METADATA) => {
            state.metadata = match event.data {
                Some(Value::Object(map)) => metadata_from_value_map(map),
                _ => HashMap::new(),
            };
            true
        }
        Some("path") | None if event.id == Some(OBSERVE_PATH) => {
            state.path = clean_string_value(event.data);
            true
        }
        Some("pause") | None if event.id == Some(OBSERVE_PAUSE) => {
            state.pause = bool_value(event.data);
            false
        }
        Some("idle-active") | None if event.id == Some(OBSERVE_IDLE_ACTIVE) => {
            state.idle_active = bool_value(event.data);
            false
        }
        Some("playlist-pos") | None if event.id == Some(OBSERVE_PLAYLIST_POS) => {
            state.playlist_pos = i64_value(event.data);
            false
        }
        Some("playlist-count") | None if event.id == Some(OBSERVE_PLAYLIST_COUNT) => {
            state.playlist_count = i64_value(event.data);
            false
        }
        Some("time-pos") | None if event.id == Some(OBSERVE_TIME_POS) => {
            state.position_seconds = number_value(event.data);
            false
        }
        Some(_) | None => false,
    };
    refresh_normalized_metadata(&mut state);
    metadata_changed
}

fn handle_end_file(state: &SharedPlaybackState) {
    let mut state = state.lock().expect("playback state lock poisoned");
    state.current_source_url = None;
    state.current_play_url = None;
    state.current_source = None;
    state.path = None;
    state.media_title = None;
    state.duration_seconds = None;
    state.position_seconds = None;
    state.metadata.clear();
    state.normalized_metadata = MediaMetadata::default();
}

fn apply_snapshot(state: &mut ObservedPlaybackState, snapshot: MetadataSnapshot) {
    state.media_title = snapshot.media_title;
    state.duration_seconds = snapshot.duration_seconds;
    state.metadata = snapshot.metadata;
    state.path = snapshot.path;
    state.pause = snapshot.pause;
    state.idle_active = snapshot.idle_active;
    state.playlist_pos = snapshot.playlist_pos;
    state.playlist_count = snapshot.playlist_count;
    state.position_seconds = snapshot.position_seconds;
}

fn take_matching_pending(state: &mut ObservedPlaybackState) -> Option<PendingPlayback> {
    let path = state.path.as_deref();
    let index = path.and_then(|path| {
        state
            .pending
            .iter()
            .position(|pending| pending.play_url == path || pending.source_url == path)
    });

    match index {
        Some(index) => state.pending.remove(index),
        None => state.pending.pop_front(),
    }
}

fn refresh_normalized_metadata(state: &mut ObservedPlaybackState) {
    let mut metadata = normalize_metadata(
        state.current_source_url.as_deref(),
        state.path.as_deref(),
        state.media_title.as_deref(),
        state.duration_seconds,
        &state.metadata,
    );
    preserve_useful_metadata(&state.normalized_metadata, &mut metadata);
    state.normalized_metadata = metadata;
}

pub fn normalize_metadata(
    source_url: Option<&str>,
    playback_path: Option<&str>,
    media_title: Option<&str>,
    duration_seconds: Option<f64>,
    metadata: &HashMap<String, String>,
) -> MediaMetadata {
    let title = first_clean(metadata, &["title", "track"])
        .or_else(|| useful_media_title(media_title, source_url, playback_path));
    let artist = first_clean(
        metadata,
        &["artist", "album_artist", "uploader", "channel", "author"],
    );
    let uploader = first_clean(metadata, &["uploader", "channel", "author"]);
    let album = first_clean(metadata, &["album"]);
    let thumbnail_url = first_clean(metadata, &["thumbnail", "thumbnail_url"]);

    MediaMetadata {
        source_url: source_url.and_then(clean_str),
        playback_path: playback_path.and_then(clean_str),
        title,
        artist,
        uploader,
        album,
        duration_seconds,
        thumbnail_url,
    }
}

fn preserve_useful_metadata(existing: &MediaMetadata, next: &mut MediaMetadata) {
    if next.title.is_none() {
        next.title = existing.title.clone();
    }
    if next.artist.is_none() {
        next.artist = existing.artist.clone();
    }
    if next.uploader.is_none() {
        next.uploader = existing.uploader.clone();
    }
    if next.album.is_none() {
        next.album = existing.album.clone();
    }
    if next.duration_seconds.is_none() {
        next.duration_seconds = existing.duration_seconds;
    }
    if next.thumbnail_url.is_none() {
        next.thumbnail_url = existing.thumbnail_url.clone();
    }
}

fn first_clean(metadata: &HashMap<String, String>, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        metadata
            .get(*key)
            .or_else(|| metadata.get(&key.to_ascii_lowercase()))
            .and_then(|value| clean_str(value))
    })
}

fn useful_media_title(
    media_title: Option<&str>,
    source_url: Option<&str>,
    playback_path: Option<&str>,
) -> Option<String> {
    let title = media_title.and_then(clean_str)?;
    if source_url.is_some_and(|source_url| equivalent_value(&title, source_url))
        || playback_path.is_some_and(|path| equivalent_value(&title, path))
    {
        return None;
    }
    Some(title)
}

fn equivalent_value(left: &str, right: &str) -> bool {
    left.trim() == right.trim()
}

fn metadata_from_value_map(map: serde_json::Map<String, Value>) -> HashMap<String, String> {
    map.into_iter()
        .filter_map(|(key, value)| match value {
            Value::String(value) => {
                clean_str(&value).map(|value| (key.to_ascii_lowercase(), value))
            }
            Value::Number(value) => Some((key.to_ascii_lowercase(), value.to_string())),
            _ => None,
        })
        .collect()
}

fn clean_string_value(value: Option<Value>) -> Option<String> {
    match value {
        Some(Value::String(value)) => clean_str(&value),
        _ => None,
    }
}

fn clean_str(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

fn number_value(value: Option<Value>) -> Option<f64> {
    match value {
        Some(Value::Number(value)) => value.as_f64(),
        _ => None,
    }
}

fn i64_value(value: Option<Value>) -> Option<i64> {
    match value {
        Some(Value::Number(value)) => value.as_i64(),
        _ => None,
    }
}

fn bool_value(value: Option<Value>) -> Option<bool> {
    match value {
        Some(Value::Bool(value)) => Some(value),
        _ => None,
    }
}

fn normalize_loop_status(loop_file: Option<String>, loop_playlist: Option<String>) -> LoopStatus {
    match (loop_file.as_deref(), loop_playlist.as_deref()) {
        (Some("inf"), _) => LoopStatus::One,
        (_, Some("inf")) => LoopStatus::Queue,
        (Some("no"), Some("no")) => LoopStatus::Off,
        _ => LoopStatus::Custom {
            loop_file,
            loop_playlist,
        },
    }
}

fn parse_response(response: &str) -> Result<MpvResponse> {
    let response: MpvResponse = serde_json::from_str(response)
        .map_err(|_| UraError::InvalidMpvResponse(response.trim().to_string()))?;
    if response.error == "success" {
        Ok(response)
    } else {
        Err(UraError::MpvCommandFailed(response.error).into())
    }
}

fn is_property_unavailable(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<UraError>()
        .map(|error| matches!(error, UraError::MpvCommandFailed(message) if message == "property unavailable"))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        os::unix::net::UnixListener,
        time::{SystemTime, UNIX_EPOCH},
    };

    #[test]
    fn builds_replace_loadfile_command() {
        assert_eq!(
            loadfile_command("https://youtu.be/example", LoadMode::Replace),
            json!({ "command": ["loadfile", "https://youtu.be/example", "replace"] })
        );
    }

    #[test]
    fn builds_enqueue_loadfile_command() {
        assert_eq!(
            loadfile_command("https://youtu.be/example", LoadMode::AppendPlay),
            json!({ "command": ["loadfile", "https://youtu.be/example", "append-play"] })
        );
    }

    #[test]
    fn builds_loop_off_commands() {
        assert_eq!(
            loop_mode_commands(LoopMode::Off),
            [
                json!({ "command": ["set_property", "loop-file", "no"] }),
                json!({ "command": ["set_property", "loop-playlist", "no"] }),
            ]
        );
    }

    #[test]
    fn builds_loop_one_commands() {
        assert_eq!(
            loop_mode_commands(LoopMode::One),
            [
                json!({ "command": ["set_property", "loop-file", "inf"] }),
                json!({ "command": ["set_property", "loop-playlist", "no"] }),
            ]
        );
    }

    #[test]
    fn builds_loop_queue_commands() {
        assert_eq!(
            loop_mode_commands(LoopMode::Queue),
            [
                json!({ "command": ["set_property", "loop-file", "no"] }),
                json!({ "command": ["set_property", "loop-playlist", "inf"] }),
            ]
        );
    }

    #[test]
    fn normalizes_loop_status() {
        assert_eq!(
            normalize_loop_status(Some("no".to_string()), Some("no".to_string())),
            LoopStatus::Off
        );
        assert_eq!(
            normalize_loop_status(Some("inf".to_string()), Some("no".to_string())),
            LoopStatus::One
        );
        assert_eq!(
            normalize_loop_status(Some("no".to_string()), Some("inf".to_string())),
            LoopStatus::Queue
        );
        assert_eq!(
            normalize_loop_status(Some("2".to_string()), Some("no".to_string())),
            LoopStatus::Custom {
                loop_file: Some("2".to_string()),
                loop_playlist: Some("no".to_string()),
            }
        );
    }

    #[test]
    fn parses_success_response() {
        let response =
            parse_response(r#"{"error":"success","data":true}"#).expect("parse success response");

        assert_eq!(response.error, "success");
        assert_eq!(response.data, Some(Value::Bool(true)));
    }

    #[test]
    fn rejects_mpv_error_response() {
        let error = parse_response(r#"{"error":"property unavailable"}"#)
            .expect_err("mpv error should fail");

        assert!(error.to_string().contains("mpv command failed"));
    }

    #[test]
    fn detects_unavailable_property_errors() {
        let error: anyhow::Error =
            UraError::MpvCommandFailed("property unavailable".to_string()).into();

        assert!(is_property_unavailable(&error));
    }

    #[test]
    fn unsupported_control_command_is_clear() {
        let error = UraError::UnsupportedControlCommand("next".to_string());

        assert_eq!(error.to_string(), "unsupported control command `next`");
    }

    #[test]
    fn command_response_skips_async_events() {
        let socket_path = unique_socket_path("command-events");
        let listener = UnixListener::bind(&socket_path).expect("bind fake mpv socket");
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept mpv client");
            let mut reader = BufReader::new(stream.try_clone().expect("clone stream"));
            let mut request = String::new();
            reader.read_line(&mut request).expect("read command");
            let request: Value = serde_json::from_str(&request).expect("parse command");
            let request_id = request
                .get("request_id")
                .and_then(Value::as_u64)
                .expect("request id");
            writeln!(
                stream,
                r#"{{"event":"property-change","id":1,"name":"media-title","data":"Example song"}}"#
            )
            .expect("write async event");
            writeln!(stream, r#"{{"request_id":{request_id},"error":"success"}}"#)
                .expect("write response");
        });

        let mut client = MpvClient::connect(&socket_path).expect("connect fake mpv");
        client
            .toggle()
            .expect("event should not be parsed as response");

        handle.join().expect("join fake mpv");
        let _ = fs::remove_file(socket_path);
    }

    #[test]
    fn replace_load_starts_playback_when_previously_paused() {
        let socket_path = unique_socket_path("replace-unpauses");
        let listener = UnixListener::bind(&socket_path).expect("bind fake mpv socket");
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept mpv client");
            let mut reader = BufReader::new(stream.try_clone().expect("clone stream"));
            let mut commands = Vec::new();
            for _ in 0..2 {
                let mut request = String::new();
                reader.read_line(&mut request).expect("read command");
                let request: Value = serde_json::from_str(&request).expect("parse command");
                let request_id = request
                    .get("request_id")
                    .and_then(Value::as_u64)
                    .expect("request id");
                commands.push(request.get("command").cloned().expect("command"));
                writeln!(stream, r#"{{"request_id":{request_id},"error":"success"}}"#)
                    .expect("write response");
            }
            commands
        });

        let mut client = MpvClient::connect(&socket_path).expect("connect fake mpv");
        client
            .load_replace("https://youtu.be/example")
            .expect("replace load");

        assert_eq!(
            handle.join().expect("join fake mpv"),
            vec![
                json!(["loadfile", "https://youtu.be/example", "replace"]),
                json!(["set_property", "pause", false]),
            ]
        );
        let _ = fs::remove_file(socket_path);
    }

    #[test]
    fn normalizes_case_insensitive_metadata() {
        let metadata = HashMap::from([
            ("title".to_string(), "Example song".to_string()),
            ("artist".to_string(), "Example artist".to_string()),
            ("uploader".to_string(), "Example uploader".to_string()),
        ]);

        let normalized = normalize_metadata(
            Some("https://youtu.be/example"),
            Some("https://youtu.be/example"),
            Some("https://youtu.be/example"),
            Some(222.5),
            &metadata,
        );

        assert_eq!(normalized.title.as_deref(), Some("Example song"));
        assert_eq!(normalized.artist.as_deref(), Some("Example artist"));
        assert_eq!(normalized.uploader.as_deref(), Some("Example uploader"));
        assert_eq!(normalized.duration_seconds, Some(222.5));
    }

    #[test]
    fn media_title_matching_source_url_is_not_authoritative_title() {
        let normalized = normalize_metadata(
            Some("https://youtu.be/example"),
            Some("https://youtu.be/example"),
            Some("https://youtu.be/example"),
            None,
            &HashMap::new(),
        );

        assert_eq!(normalized.title, None);
    }

    #[test]
    fn property_changes_enrich_current_metadata() {
        let state = shared_playback_state();
        assert!(handle_property_change(
            event_fixture(
                OBSERVE_MEDIA_TITLE,
                "media-title",
                Value::String("Example song".to_string()),
            ),
            &state,
        ));
        assert!(handle_property_change(
            event_fixture(
                OBSERVE_DURATION,
                "duration",
                Value::Number(serde_json::Number::from_f64(222.5).expect("number")),
            ),
            &state,
        ));
        assert!(handle_property_change(
            event_fixture(
                OBSERVE_METADATA,
                "metadata",
                json!({"TITLE":"Example song","ARTIST":"Example artist"}),
            ),
            &state,
        ));

        let state = state.lock().expect("state lock");
        assert_eq!(
            state.normalized_metadata.title.as_deref(),
            Some("Example song")
        );
        assert_eq!(
            state.normalized_metadata.artist.as_deref(),
            Some("Example artist")
        );
        assert_eq!(state.normalized_metadata.duration_seconds, Some(222.5));
    }

    #[test]
    fn runtime_property_changes_update_state_without_metadata_effect() {
        let state = shared_playback_state();

        assert!(!handle_property_change(
            event_fixture(
                OBSERVE_TIME_POS,
                "time-pos",
                Value::Number(serde_json::Number::from_f64(12.5).expect("number")),
            ),
            &state,
        ));
        assert!(!handle_property_change(
            event_fixture(OBSERVE_PAUSE, "pause", Value::Bool(true)),
            &state,
        ));
        assert!(!handle_property_change(
            event_fixture(
                OBSERVE_PLAYLIST_POS,
                "playlist-pos",
                Value::Number(serde_json::Number::from(1)),
            ),
            &state,
        ));
        assert!(!handle_property_change(
            event_fixture(
                OBSERVE_PLAYLIST_COUNT,
                "playlist-count",
                Value::Number(serde_json::Number::from(3)),
            ),
            &state,
        ));

        let state = state.lock().expect("state lock");
        assert_eq!(state.position_seconds, Some(12.5));
        assert_eq!(state.pause, Some(true));
        assert_eq!(state.playlist_pos, Some(1));
        assert_eq!(state.playlist_count, Some(3));
    }

    #[test]
    fn metadata_relevant_properties_report_metadata_effect() {
        let state = shared_playback_state();

        assert!(handle_property_change(
            event_fixture(
                OBSERVE_MEDIA_TITLE,
                "media-title",
                Value::String("Example song".to_string()),
            ),
            &state,
        ));
        assert!(handle_property_change(
            event_fixture(
                OBSERVE_DURATION,
                "duration",
                Value::Number(serde_json::Number::from_f64(10.0).expect("number")),
            ),
            &state,
        ));
        assert!(handle_property_change(
            event_fixture(
                OBSERVE_METADATA,
                "metadata",
                json!({"TITLE":"Metadata title"})
            ),
            &state,
        ));
        assert!(handle_property_change(
            event_fixture(
                OBSERVE_PATH,
                "path",
                Value::String("https://youtu.be/example".to_string()),
            ),
            &state,
        ));
    }

    #[test]
    fn icy_bitrate_is_not_thumbnail_metadata() {
        let metadata = HashMap::from([("icy-br".to_string(), "128".to_string())]);

        let normalized = normalize_metadata(None, None, None, None, &metadata);

        assert_eq!(normalized.thumbnail_url, None);
    }

    #[test]
    fn thumbnail_metadata_keys_are_accepted() {
        let metadata = HashMap::from([(
            "thumbnail".to_string(),
            "https://example.test/thumb.jpg".to_string(),
        )]);
        let normalized = normalize_metadata(None, None, None, None, &metadata);
        assert_eq!(
            normalized.thumbnail_url.as_deref(),
            Some("https://example.test/thumb.jpg")
        );

        let metadata = HashMap::from([(
            "thumbnail_url".to_string(),
            "https://example.test/thumb-url.jpg".to_string(),
        )]);
        let normalized = normalize_metadata(None, None, None, None, &metadata);
        assert_eq!(
            normalized.thumbnail_url.as_deref(),
            Some("https://example.test/thumb-url.jpg")
        );
    }

    #[test]
    fn file_loaded_records_matching_queued_track() {
        let path = unique_db_path("queued-track");
        let database = Database::open(path.clone()).expect("open database");
        let state = shared_playback_state();
        register_playback_request(
            &state,
            "https://youtu.be/first".to_string(),
            "https://youtu.be/first".to_string(),
            Some("cli".to_string()),
            QueueMode::Append,
        );
        register_playback_request(
            &state,
            "https://youtu.be/second".to_string(),
            "https://youtu.be/second".to_string(),
            Some("cli".to_string()),
            QueueMode::Append,
        );

        handle_file_loaded(
            MetadataSnapshot {
                path: Some("https://youtu.be/second".to_string()),
                media_title: Some("Second song".to_string()),
                duration_seconds: Some(12.0),
                ..MetadataSnapshot::default()
            },
            &database,
            &state,
        )
        .expect("handle file loaded");

        let history = database.history().expect("read history");
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].source_url, "https://youtu.be/second");
        assert_eq!(history[0].title.as_deref(), Some("Second song"));

        let _ = fs::remove_file(path);
    }

    #[test]
    fn reconciling_an_already_recorded_file_does_not_duplicate_history() {
        let path = unique_db_path("reconciled-track");
        let database = Database::open(path.clone()).expect("open database");
        let state = shared_playback_state();
        register_playback_request(
            &state,
            "https://youtu.be/example".to_string(),
            "https://youtu.be/example".to_string(),
            Some("cli".to_string()),
            QueueMode::Replace,
        );
        let snapshot = MetadataSnapshot {
            path: Some("https://youtu.be/example".to_string()),
            media_title: Some("Example song".to_string()),
            idle_active: Some(false),
            ..MetadataSnapshot::default()
        };

        handle_file_loaded(snapshot.clone(), &database, &state).expect("initial file load");
        handle_file_loaded(snapshot, &database, &state).expect("reconciled file load");

        assert_eq!(database.history().expect("read history").len(), 1);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn later_property_change_updates_existing_track_without_new_play() {
        let path = unique_db_path("later-property");
        let database = Database::open(path.clone()).expect("open database");
        let state = shared_playback_state();
        register_playback_request(
            &state,
            "https://youtu.be/example".to_string(),
            "https://youtu.be/example".to_string(),
            Some("cli".to_string()),
            QueueMode::Append,
        );
        handle_file_loaded(
            MetadataSnapshot {
                path: Some("https://youtu.be/example".to_string()),
                ..MetadataSnapshot::default()
            },
            &database,
            &state,
        )
        .expect("handle file loaded");

        handle_observer_line(
            r#"{"event":"property-change","id":3,"name":"metadata","data":{"TITLE":"Example song","ARTIST":"Example artist"}}"#,
            Path::new("/tmp/missing.sock"),
            &database,
            &state,
        )
        .expect("handle metadata event");

        let history = database.history().expect("read history");
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].play_count, 1);
        assert_eq!(history[0].title.as_deref(), Some("Example song"));
        assert_eq!(history[0].uploader.as_deref(), Some("Example artist"));

        let _ = fs::remove_file(path);
    }

    #[test]
    fn end_file_clears_current_playback_state() {
        let state = shared_playback_state();
        handle_property_change(
            event_fixture(
                OBSERVE_MEDIA_TITLE,
                "media-title",
                Value::String("Example song".to_string()),
            ),
            &state,
        );

        handle_end_file(&state);

        let state = state.lock().expect("state lock");
        assert_eq!(state.normalized_metadata.title, None);
        assert_eq!(state.media_title, None);
    }

    #[test]
    fn unknown_events_are_ignored_and_malformed_events_error() {
        let path = unique_db_path("unknown-events");
        let database = Database::open(path.clone()).expect("open database");
        let state = shared_playback_state();

        handle_observer_line(
            r#"{"event":"unknown-event","data":true}"#,
            Path::new("/tmp/missing.sock"),
            &database,
            &state,
        )
        .expect("unknown event should be ignored");
        let error = handle_observer_line(
            "not-json",
            Path::new("/tmp/missing.sock"),
            &database,
            &state,
        )
        .expect_err("malformed event should return useful error");
        assert!(error.to_string().contains("invalid"));

        let _ = fs::remove_file(path);
    }

    #[test]
    fn observer_shutdown_joins_without_socket() {
        let path = unique_db_path("observer-shutdown");
        let database = Arc::new(Database::open(path.clone()).expect("open database"));
        let observer = MpvEventObserver::start(
            unique_socket_path("observer-missing"),
            database,
            shared_playback_state(),
        );
        drop(observer);

        let _ = fs::remove_file(path);
    }

    fn event_fixture(id: i64, name: &str, data: Value) -> MpvEventMessage {
        MpvEventMessage {
            event: Some("property-change".to_string()),
            id: Some(id),
            name: Some(name.to_string()),
            data: Some(data),
        }
    }

    fn unique_socket_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("ura-{name}-{}.sock", unique_suffix()))
    }

    fn unique_db_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("ura-{name}-{}.db", unique_suffix()))
    }

    fn unique_suffix() -> u128 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time before UNIX_EPOCH")
            .as_nanos()
    }
}
