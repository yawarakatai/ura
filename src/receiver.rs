use std::{
    fs,
    net::SocketAddr,
    path::{Path, PathBuf},
    process::{Command as StdCommand, ExitStatus, Stdio},
    sync::Arc,
};

use anyhow::{Context, Result as AnyhowResult};
use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use tokio::{net::TcpListener, process::Command as TokioCommand};

use crate::config::{default_db_path, default_mpv_socket_path};
use crate::db::{Database, HistoryEntry};
use crate::mpv::{LoopMode, LoopStatus, MpvClient};

pub async fn run_receive(bind: SocketAddr, token: String) -> AnyhowResult<()> {
    ensure_program_in_path("mpv")?;
    ensure_program_in_path("yt-dlp")?;

    let socket_path = default_mpv_socket_path()?;
    create_runtime_dir(&socket_path)?;
    let database = Database::open(default_db_path()?)?;

    let mut receiver = Receiver::start(socket_path)?;
    println!(
        "receive: mpv started with IPC socket at {}; HTTP API listening on http://{}",
        receiver.socket_path.display(),
        bind
    );

    let api = run_api(bind, token, receiver.socket_path.clone(), database);
    tokio::select! {
        result = receiver.wait() => {
            let status = result?;
            if status.success() {
                Ok(())
            } else {
                Err(anyhow::anyhow!("mpv exited with status {status}"))
            }
        }
        result = api => result,
    }
}

async fn run_api(
    bind: SocketAddr,
    token: String,
    socket_path: PathBuf,
    database: Database,
) -> AnyhowResult<()> {
    let state = AppState {
        token: Arc::from(token),
        socket_path: Arc::new(socket_path),
        database: Arc::new(database),
    };
    let app = app(state);
    let listener = TcpListener::bind(bind)
        .await
        .with_context(|| format!("failed to bind HTTP receiver to {bind}"))?;

    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
        .with_context(|| "HTTP receiver failed")
}

fn app(state: AppState) -> Router {
    Router::new()
        .route("/v1/play", post(play))
        .route("/v1/enqueue", post(enqueue))
        .route("/v1/control", post(control))
        .route("/v1/status", get(status))
        .route("/v1/history", get(history))
        .with_state(state)
}

#[derive(Clone)]
struct AppState {
    token: Arc<str>,
    socket_path: Arc<PathBuf>,
    database: Arc<Database>,
}

#[derive(Debug, Deserialize)]
struct PlayRequest {
    url: String,
    source: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ControlRequest {
    command: String,
}

#[derive(Debug, Serialize)]
struct OkResponse {
    ok: bool,
}

#[derive(Debug, Serialize)]
struct ControlResponse {
    ok: bool,
    loop_status: Option<LoopStatus>,
}

#[derive(Debug, Serialize)]
struct ErrorResponse {
    error: String,
}

#[derive(Debug)]
struct AppError {
    status: StatusCode,
    message: String,
}

impl AppError {
    fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(ErrorResponse {
                error: self.message,
            }),
        )
            .into_response()
    }
}

async fn play(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<PlayRequest>,
) -> std::result::Result<Json<OkResponse>, AppError> {
    authorize(&headers, &state)?;
    validate_supported_url(&request.url)?;
    let url = request.url;
    let source = request.source;

    run_mpv_command(state.clone(), {
        let url = url.clone();
        move |client| client.load_replace(&url)
    })
    .await?;
    record_history(state, url, source).await?;
    Ok(Json(OkResponse { ok: true }))
}

async fn enqueue(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<PlayRequest>,
) -> std::result::Result<Json<OkResponse>, AppError> {
    authorize(&headers, &state)?;
    validate_supported_url(&request.url)?;
    let url = request.url;
    let source = request.source;

    run_mpv_command(state.clone(), {
        let url = url.clone();
        move |client| client.load_enqueue(&url)
    })
    .await?;
    record_history(state, url, source).await?;
    Ok(Json(OkResponse { ok: true }))
}

async fn control(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<ControlRequest>,
) -> std::result::Result<Json<ControlResponse>, AppError> {
    authorize(&headers, &state)?;
    let command = request.command;

    let loop_status = match command.as_str() {
        "toggle" | "stop" | "pause" | "resume" => {
            run_mpv_command(state, move |client| client.control(&command)).await?;
            None
        }
        "loop-off" => {
            run_mpv_command(state, |client| client.set_loop_mode(LoopMode::Off)).await?;
            None
        }
        "loop-one" => {
            run_mpv_command(state, |client| client.set_loop_mode(LoopMode::One)).await?;
            None
        }
        "loop-queue" => {
            run_mpv_command(state, |client| client.set_loop_mode(LoopMode::Queue)).await?;
            None
        }
        "loop-status" => Some(run_mpv_command(state, |client| client.loop_status()).await?),
        other => {
            return Err(AppError::new(
                StatusCode::BAD_REQUEST,
                format!("unsupported control command `{other}`"),
            ));
        }
    };

    Ok(Json(ControlResponse {
        ok: true,
        loop_status,
    }))
}

async fn status(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> std::result::Result<Json<crate::mpv::MpvStatus>, AppError> {
    authorize(&headers, &state)?;
    let status = run_mpv_command(state, |client| client.status()).await?;
    Ok(Json(status))
}

async fn history(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> std::result::Result<Json<Vec<HistoryEntry>>, AppError> {
    authorize(&headers, &state)?;
    let history = run_db_command(state, |database| database.history()).await?;
    Ok(Json(history))
}

async fn run_mpv_command<T, F>(state: AppState, command: F) -> std::result::Result<T, AppError>
where
    T: Send + 'static,
    F: FnOnce(&mut MpvClient) -> AnyhowResult<T> + Send + 'static,
{
    tokio::task::spawn_blocking(move || {
        let mut client = MpvClient::connect(&state.socket_path)?;
        command(&mut client)
    })
    .await
    .map_err(|error| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?
    .map_err(|error| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))
}

async fn record_history(
    state: AppState,
    url: String,
    source: Option<String>,
) -> std::result::Result<(), AppError> {
    run_db_command(state, move |database| {
        database.record_play(&url, source.as_deref())
    })
    .await
}

async fn run_db_command<T, F>(state: AppState, command: F) -> std::result::Result<T, AppError>
where
    T: Send + 'static,
    F: FnOnce(&Database) -> AnyhowResult<T> + Send + 'static,
{
    tokio::task::spawn_blocking(move || command(&state.database))
        .await
        .map_err(|error| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?
        .map_err(|error| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))
}

fn authorize(headers: &HeaderMap, state: &AppState) -> std::result::Result<(), AppError> {
    let expected = format!("Bearer {}", state.token);
    let Some(actual) = headers.get(axum::http::header::AUTHORIZATION) else {
        return Err(AppError::new(
            StatusCode::UNAUTHORIZED,
            "missing bearer token",
        ));
    };

    if actual.as_bytes() == expected.as_bytes() {
        Ok(())
    } else {
        Err(AppError::new(
            StatusCode::UNAUTHORIZED,
            "invalid bearer token",
        ))
    }
}

fn validate_supported_url(url: &str) -> std::result::Result<(), AppError> {
    let Some(parsed) = ParsedUrl::parse(url) else {
        Err(AppError::new(
            StatusCode::BAD_REQUEST,
            "unsupported URL; expected a YouTube URL using http:// or https://",
        ))?
    };

    if parsed.has_playlist() {
        return Err(AppError::new(
            StatusCode::BAD_REQUEST,
            "playlist URLs are not supported in the MVP",
        ));
    }

    if parsed.is_supported_youtube_url() {
        Ok(())
    } else {
        Err(AppError::new(
            StatusCode::BAD_REQUEST,
            "unsupported URL; expected youtube.com, music.youtube.com, or youtu.be",
        ))
    }
}

#[derive(Debug, PartialEq, Eq)]
struct ParsedUrl {
    host: String,
    path: String,
    query: Option<String>,
}

impl ParsedUrl {
    fn parse(url: &str) -> Option<Self> {
        let url = url.trim();
        if url.is_empty() {
            return None;
        }

        let (scheme, rest) = url.split_once("://")?;
        let scheme = scheme.to_ascii_lowercase();
        if scheme != "http" && scheme != "https" {
            return None;
        }

        let (authority, path_and_query) = match rest.split_once('/') {
            Some((authority, path_and_query)) => (authority, format!("/{path_and_query}")),
            None => (rest, "/".to_string()),
        };
        if authority.is_empty() {
            return None;
        }

        let host = authority
            .rsplit('@')
            .next()
            .unwrap_or(authority)
            .split(':')
            .next()
            .unwrap_or(authority)
            .to_ascii_lowercase();
        let (path, query) = match path_and_query.split_once('?') {
            Some((path, query)) => (path.to_string(), Some(query.to_string())),
            None => (path_and_query, None),
        };

        Some(Self { host, path, query })
    }

    fn is_supported_youtube_url(&self) -> bool {
        match self.host.as_str() {
            "youtube.com" | "www.youtube.com" | "music.youtube.com" => self.path == "/watch",
            "youtu.be" => !self.path.trim_start_matches('/').is_empty(),
            _ => false,
        }
    }

    fn has_playlist(&self) -> bool {
        self.path == "/playlist"
            || self
                .query
                .as_deref()
                .map(|query| {
                    query.split('&').any(|part| {
                        part.split_once('=')
                            .map(|(name, _)| name == "list")
                            .unwrap_or(part == "list")
                    })
                })
                .unwrap_or(false)
    }
}

struct Receiver {
    mpv: tokio::process::Child,
    socket_path: PathBuf,
}

impl Receiver {
    fn start(socket_path: PathBuf) -> AnyhowResult<Self> {
        let mpv = TokioCommand::new("mpv")
            .args(mpv_args(&socket_path))
            .stdin(Stdio::null())
            .spawn()
            .with_context(|| "failed to start mpv")?;

        Ok(Self { mpv, socket_path })
    }

    async fn wait(&mut self) -> AnyhowResult<ExitStatus> {
        self.mpv
            .wait()
            .await
            .with_context(|| "failed to wait for mpv child process")
    }
}

impl Drop for Receiver {
    fn drop(&mut self) {
        if let Ok(None) = self.mpv.try_wait() {
            let _ = self.mpv.start_kill();
        }
    }
}

fn create_runtime_dir(socket_path: &Path) -> AnyhowResult<()> {
    let runtime_dir = socket_path
        .parent()
        .expect("default mpv socket path should include a runtime directory");
    fs::create_dir_all(runtime_dir).with_context(|| {
        format!(
            "failed to create runtime directory {}",
            runtime_dir.display()
        )
    })
}

fn ensure_program_in_path(program: &str) -> AnyhowResult<()> {
    let status = StdCommand::new(program)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .with_context(|| format!("failed to find `{program}` in PATH"))?;

    if status.success() {
        Ok(())
    } else {
        Err(anyhow::anyhow!(
            "`{program} --version` failed with status {status}"
        ))
    }
}

fn mpv_args(socket_path: &Path) -> Vec<String> {
    vec![
        "--idle=yes".to_string(),
        "--no-video".to_string(),
        "--force-window=no".to_string(),
        "--terminal=no".to_string(),
        format!("--input-ipc-server={}", socket_path.display()),
        "--ytdl-format=bestaudio/best".to_string(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{HeaderMap, HeaderValue, header::AUTHORIZATION};

    fn unique_path(name: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time before UNIX_EPOCH")
            .as_nanos();
        std::env::temp_dir().join(format!("ura-{name}-{nanos}"))
    }

    fn test_state() -> (AppState, PathBuf) {
        let db_path = unique_path("receiver-history.db");
        let state = AppState {
            token: Arc::from("secret"),
            socket_path: Arc::new(PathBuf::from("/tmp/ura.sock")),
            database: Arc::new(Database::open(db_path.clone()).expect("open test database")),
        };
        (state, db_path)
    }

    #[test]
    fn builds_audio_only_mpv_startup_args() {
        let socket_path = Path::new("/run/user/1000/ura/mpv.sock");

        assert_eq!(
            mpv_args(socket_path),
            vec![
                "--idle=yes",
                "--no-video",
                "--force-window=no",
                "--terminal=no",
                "--input-ipc-server=/run/user/1000/ura/mpv.sock",
                "--ytdl-format=bestaudio/best",
            ]
        );
    }

    #[test]
    fn creates_runtime_directory_for_socket() {
        let runtime_dir = unique_path("runtime-dir");
        let socket_path = runtime_dir.join("ura/mpv.sock");

        create_runtime_dir(&socket_path).expect("create runtime directory");

        assert!(runtime_dir.join("ura").is_dir());

        let _ = fs::remove_dir_all(runtime_dir);
    }

    #[test]
    fn accepts_valid_bearer_token() {
        let (state, db_path) = test_state();
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, HeaderValue::from_static("Bearer secret"));

        authorize(&headers, &state).expect("valid token should pass");

        let _ = fs::remove_file(db_path);
    }

    #[test]
    fn rejects_missing_bearer_token() {
        let (state, db_path) = test_state();

        let error = authorize(&HeaderMap::new(), &state).expect_err("missing token should fail");

        assert_eq!(error.status, StatusCode::UNAUTHORIZED);

        let _ = fs::remove_file(db_path);
    }

    #[test]
    fn rejects_invalid_bearer_token() {
        let (state, db_path) = test_state();
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, HeaderValue::from_static("Bearer wrong"));

        let error = authorize(&headers, &state).expect_err("invalid token should fail");

        assert_eq!(error.status, StatusCode::UNAUTHORIZED);

        let _ = fs::remove_file(db_path);
    }

    #[test]
    fn accepts_supported_youtube_urls() {
        validate_supported_url("https://www.youtube.com/watch?v=example")
            .expect("www youtube watch URL should pass");
        validate_supported_url("https://youtube.com/watch?v=example")
            .expect("youtube watch URL should pass");
        validate_supported_url("https://music.youtube.com/watch?v=example")
            .expect("music youtube watch URL should pass");
        validate_supported_url("https://youtu.be/example").expect("https should pass");
    }

    #[test]
    fn rejects_unsafe_url_schemes() {
        let error = validate_supported_url("file:///tmp/audio").expect_err("file URL should fail");

        assert_eq!(error.status, StatusCode::BAD_REQUEST);
    }

    #[test]
    fn rejects_unsupported_domains() {
        let error =
            validate_supported_url("https://example.test/watch?v=example").expect_err("domain");

        assert_eq!(error.status, StatusCode::BAD_REQUEST);
    }

    #[test]
    fn rejects_empty_urls() {
        let error = validate_supported_url("").expect_err("empty URL should fail");

        assert_eq!(error.status, StatusCode::BAD_REQUEST);
    }

    #[test]
    fn rejects_playlist_urls() {
        let watch_playlist =
            validate_supported_url("https://www.youtube.com/watch?v=example&list=playlist")
                .expect_err("watch playlist should fail");
        let playlist = validate_supported_url("https://www.youtube.com/playlist?list=playlist")
            .expect_err("playlist should fail");

        assert_eq!(watch_playlist.status, StatusCode::BAD_REQUEST);
        assert_eq!(playlist.status, StatusCode::BAD_REQUEST);
    }
}
