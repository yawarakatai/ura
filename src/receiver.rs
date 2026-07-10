use std::{
    fs,
    io::ErrorKind,
    net::SocketAddr,
    os::unix::{fs::FileTypeExt, net::UnixStream},
    path::{Path, PathBuf},
    process::{Command as StdCommand, ExitStatus, Stdio},
    sync::Arc,
};

use anyhow::{Context, Result as AnyhowResult};
use axum::{
    Json, Router,
    extract::{Request, State},
    http::{HeaderMap, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use tokio::{
    net::TcpListener,
    process::{Child, Command as TokioCommand},
    sync::oneshot,
    task::JoinHandle,
};
use tracing::{error, info, warn};

use crate::config::{default_db_path, default_mpv_socket_path};
use crate::db::{Database, HistoryEntry};
use crate::mpv::{LoopMode, LoopStatus, MpvClient};

pub async fn run_serve(bind: SocketAddr, token: String) -> AnyhowResult<()> {
    info!("receiver startup");
    validate_receiver_token(&token)?;
    ensure_program_in_path("mpv")?;
    ensure_program_in_path("yt-dlp")?;

    let socket_path = default_mpv_socket_path()?;
    info!(socket_path = %socket_path.display(), "mpv IPC socket path");
    create_runtime_dir(&socket_path)?;
    let database = Database::open(default_db_path()?)?;

    let receiver = Receiver::start(socket_path)?;
    let (socket_path, mut mpv_shutdown, mut mpv_task) = receiver.into_parts();
    info!(bind = %bind, "HTTP receiver bind address");

    let (api_shutdown, api_shutdown_rx) = oneshot::channel();
    let mut api_shutdown = Some(api_shutdown);
    let mut api_task = tokio::spawn(run_api(bind, token, socket_path, database, api_shutdown_rx));

    tokio::select! {
        result = &mut mpv_task => {
            let status = task_result(result, "mpv supervision")?;
            info!(status = %status, "mpv child termination");
            signal_shutdown(&mut api_shutdown);
            await_task(api_task, "HTTP receiver").await?;
            info!("receiver shutdown");
            exit_status_result(status)
        }
        result = &mut api_task => {
            let api_result = task_result(result, "HTTP receiver");
            signal_shutdown(&mut mpv_shutdown);
            let mpv_result = await_task(mpv_task, "mpv supervision").await;
            api_result?;
            let status = mpv_result?;
            info!(status = %status, "mpv child termination");
            info!("receiver shutdown");
            Ok(())
        }
        result = tokio::signal::ctrl_c() => {
            info!("shutdown signal received");
            let ctrl_c_result = result.with_context(|| "failed to listen for Ctrl+C");
            signal_shutdown(&mut api_shutdown);
            signal_shutdown(&mut mpv_shutdown);
            let mpv_result = await_task(mpv_task, "mpv supervision").await;
            let api_result = await_task(api_task, "HTTP receiver").await;
            ctrl_c_result?;
            let status = mpv_result?;
            api_result?;
            info!(status = %status, "mpv child termination");
            info!("receiver shutdown");
            Ok(())
        }
    }
}

async fn run_api(
    bind: SocketAddr,
    token: String,
    socket_path: PathBuf,
    database: Database,
    shutdown: oneshot::Receiver<()>,
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
    info!(bind = %bind, "HTTP receiver listening");

    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = shutdown.await;
        })
        .await
        .with_context(|| "HTTP receiver failed")
}

fn app(state: AppState) -> Router {
    let auth_state = state.clone();
    Router::new()
        .route("/v1/play", post(play))
        .route("/v1/enqueue", post(enqueue))
        .route("/v1/control", post(control))
        .route("/v1/status", get(status))
        .route("/v1/history", get(history))
        .route_layer(middleware::from_fn_with_state(auth_state, require_auth))
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
    Json(request): Json<PlayRequest>,
) -> std::result::Result<Json<OkResponse>, AppError> {
    info!(request_type = "play", "HTTP request");
    let play_url = validate_supported_url(&request.url)?;
    let source_url = request.url;
    let source = request.source;

    run_mpv_command(state.clone(), {
        let play_url = play_url.clone();
        move |client| client.load_replace(&play_url)
    })
    .await?;
    record_history(state, source_url, play_url, source).await?;
    Ok(Json(OkResponse { ok: true }))
}

async fn enqueue(
    State(state): State<AppState>,
    Json(request): Json<PlayRequest>,
) -> std::result::Result<Json<OkResponse>, AppError> {
    info!(request_type = "enqueue", "HTTP request");
    let play_url = validate_supported_url(&request.url)?;
    let source_url = request.url;
    let source = request.source;

    run_mpv_command(state.clone(), {
        let play_url = play_url.clone();
        move |client| client.load_enqueue(&play_url)
    })
    .await?;
    record_history(state, source_url, play_url, source).await?;
    Ok(Json(OkResponse { ok: true }))
}

async fn control(
    State(state): State<AppState>,
    Json(request): Json<ControlRequest>,
) -> std::result::Result<Json<ControlResponse>, AppError> {
    let command = request.command;
    info!(request_type = "control", command = %command, "HTTP request");

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
) -> std::result::Result<Json<crate::mpv::MpvStatus>, AppError> {
    info!(request_type = "status", "HTTP request");
    let status = run_mpv_command(state, |client| client.status()).await?;
    Ok(Json(status))
}

async fn history(
    State(state): State<AppState>,
) -> std::result::Result<Json<Vec<HistoryEntry>>, AppError> {
    info!(request_type = "history", "HTTP request");
    let history = run_db_command(state, |database| database.history()).await?;
    Ok(Json(history))
}

async fn require_auth(
    State(state): State<AppState>,
    headers: HeaderMap,
    request: Request,
    next: Next,
) -> std::result::Result<Response, AppError> {
    authorize(&headers, &state)?;
    Ok(next.run(request).await)
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
    .map_err(|error| {
        error!(error = %error, "mpv IPC task failed");
        AppError::new(StatusCode::INTERNAL_SERVER_ERROR, error.to_string())
    })?
    .map_err(|error| {
        error!(error = %error, "mpv IPC error");
        AppError::new(StatusCode::INTERNAL_SERVER_ERROR, error.to_string())
    })
}

async fn record_history(
    state: AppState,
    source_url: String,
    play_url: String,
    source: Option<String>,
) -> std::result::Result<(), AppError> {
    let result = run_db_command(state, move |database| {
        database.record_play(&source_url, &play_url, source.as_deref())
    })
    .await;
    if let Err(error) = &result {
        error!(error = %error.message, "database write error");
    }
    result
}

async fn run_db_command<T, F>(state: AppState, command: F) -> std::result::Result<T, AppError>
where
    T: Send + 'static,
    F: FnOnce(&Database) -> AnyhowResult<T> + Send + 'static,
{
    tokio::task::spawn_blocking(move || command(&state.database))
        .await
        .map_err(|error| {
            error!(error = %error, "database task failed");
            AppError::new(StatusCode::INTERNAL_SERVER_ERROR, error.to_string())
        })?
        .map_err(|error| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))
}

fn authorize(headers: &HeaderMap, state: &AppState) -> std::result::Result<(), AppError> {
    let expected = format!("Bearer {}", state.token);
    let Some(actual) = headers.get(axum::http::header::AUTHORIZATION) else {
        warn!(reason = "missing bearer token", "auth failure");
        return Err(AppError::new(
            StatusCode::UNAUTHORIZED,
            "missing bearer token",
        ));
    };

    if actual.as_bytes() == expected.as_bytes() {
        Ok(())
    } else {
        warn!(reason = "invalid bearer token", "auth failure");
        Err(AppError::new(
            StatusCode::UNAUTHORIZED,
            "invalid bearer token",
        ))
    }
}

pub fn validate_receiver_token(token: &str) -> AnyhowResult<()> {
    let token = token.trim();
    if token.is_empty() {
        return Err(anyhow::anyhow!("receiver token must not be empty"));
    }
    if token == "change-me" {
        return Err(anyhow::anyhow!(
            "receiver token must be changed before starting the receiver"
        ));
    }
    if token.len() < 32 {
        return Err(anyhow::anyhow!(
            "receiver token must be at least 32 characters"
        ));
    }

    Ok(())
}

fn validate_supported_url(url: &str) -> std::result::Result<String, AppError> {
    let Some(parsed) = ParsedUrl::parse(url) else {
        log_url_validation_failure(url, "malformed URL or unsupported scheme");
        Err(AppError::new(
            StatusCode::BAD_REQUEST,
            "unsupported URL; expected a YouTube URL using http:// or https://",
        ))?
    };

    if parsed.is_supported_youtube_video_url() {
        log_url_validation_success(&parsed);
        return Ok(parsed.without_playlist_context());
    }

    if parsed.has_playlist_context() {
        log_url_validation_failure(url, "playlist URLs are not supported in the MVP");
        return Err(AppError::new(
            StatusCode::BAD_REQUEST,
            "playlist URLs are not supported in the MVP",
        ));
    }

    log_url_validation_failure(
        url,
        "unsupported host; expected youtube.com, music.youtube.com, or youtu.be",
    );
    Err(AppError::new(
        StatusCode::BAD_REQUEST,
        "unsupported URL; expected youtube.com, music.youtube.com, or youtu.be",
    ))
}

fn log_url_validation_success(parsed: &ParsedUrl) {
    match parsed.sanitized_youtube_video_id() {
        Some(video_id) => info!(
            host = %parsed.host,
            video_id = %video_id,
            "URL validation succeeded"
        ),
        None => info!(host = %parsed.host, "URL validation succeeded"),
    }
}

fn log_url_validation_failure(url: &str, reason: &'static str) {
    match safe_host_from_url(url) {
        Some(host) => warn!(host = %host, reason, "URL validation failed"),
        None => warn!(reason, "URL validation failed"),
    }
}

fn safe_host_from_url(url: &str) -> Option<String> {
    let trimmed = url.trim();
    let (_, rest) = trimmed.split_once("://")?;
    let authority_with_query = rest
        .split_once('/')
        .map(|(authority, _)| authority)
        .unwrap_or(rest);
    let authority = authority_with_query
        .split_once('?')
        .map(|(authority, _)| authority)
        .unwrap_or(authority_with_query);

    authority
        .parse::<Authority>()
        .ok()
        .map(|authority| authority.host)
}

#[derive(Debug, PartialEq, Eq)]
struct ParsedUrl {
    scheme: String,
    authority: String,
    host: String,
    path: String,
    query: Option<String>,
    fragment: Option<String>,
}

impl ParsedUrl {
    fn parse(url: &str) -> Option<Self> {
        let url = url.trim();
        if url.is_empty() || url.chars().any(char::is_whitespace) {
            return None;
        }

        let (scheme, rest) = url.split_once("://")?;
        let scheme = scheme.to_ascii_lowercase();
        if scheme != "http" && scheme != "https" {
            return None;
        }

        let (rest, fragment) = match rest.split_once('#') {
            Some((rest, fragment)) => (rest, Some(fragment.to_string())),
            None => (rest, None),
        };
        let (authority, path_and_query) = match rest.split_once('/') {
            Some((authority, path_and_query)) => (authority, format!("/{path_and_query}")),
            None => (rest, "/".to_string()),
        };
        if authority.is_empty() {
            return None;
        }

        let host = authority.parse::<Authority>().ok()?.host;
        let (path, query) = match path_and_query.split_once('?') {
            Some((path, query)) => (path.to_string(), Some(query.to_string())),
            None => (path_and_query, None),
        };

        Some(Self {
            scheme,
            authority: authority.to_string(),
            host,
            path,
            query,
            fragment,
        })
    }

    fn is_supported_youtube_video_url(&self) -> bool {
        match self.host.as_str() {
            "youtube.com" | "www.youtube.com" | "music.youtube.com" => {
                self.path == "/watch" && self.has_non_empty_query_param("v")
            }
            "youtu.be" => !self.path.trim_start_matches('/').is_empty(),
            _ => false,
        }
    }

    fn has_playlist_context(&self) -> bool {
        self.path == "/playlist" || self.has_query_param("list")
    }

    fn has_query_param(&self, name: &str) -> bool {
        self.query
            .as_deref()
            .map(|query| query.split('&').any(|part| query_param_name(part) == name))
            .unwrap_or(false)
    }

    fn has_non_empty_query_param(&self, name: &str) -> bool {
        self.query
            .as_deref()
            .map(|query| {
                query.split('&').any(|part| {
                    part.split_once('=')
                        .map(|(param_name, value)| param_name == name && !value.is_empty())
                        .unwrap_or(false)
                })
            })
            .unwrap_or(false)
    }

    fn query_param_value(&self, name: &str) -> Option<&str> {
        self.query.as_deref()?.split('&').find_map(|part| {
            part.split_once('=')
                .and_then(|(param_name, value)| (param_name == name).then_some(value))
        })
    }

    fn sanitized_youtube_video_id(&self) -> Option<String> {
        let raw = match self.host.as_str() {
            "youtube.com" | "www.youtube.com" | "music.youtube.com" => self.query_param_value("v"),
            "youtu.be" => self.path.trim_start_matches('/').split('/').next(),
            _ => None,
        }?;

        sanitize_video_id(raw)
    }

    fn without_playlist_context(&self) -> String {
        let query = self.query_without_param("list");
        let mut url = format!("{}://{}{}", self.scheme, self.authority, self.path);

        if let Some(query) = query {
            url.push('?');
            url.push_str(&query);
        }

        if let Some(fragment) = &self.fragment {
            url.push('#');
            url.push_str(fragment);
        }

        url
    }

    fn query_without_param(&self, name: &str) -> Option<String> {
        self.query
            .as_deref()
            .map(|query| {
                query
                    .split('&')
                    .filter(|part| query_param_name(part) != name)
                    .collect::<Vec<_>>()
                    .join("&")
            })
            .filter(|query| !query.is_empty())
    }
}

struct Authority {
    host: String,
}

impl std::str::FromStr for Authority {
    type Err = ();

    fn from_str(authority: &str) -> std::result::Result<Self, Self::Err> {
        if authority.contains('@') {
            return Err(());
        }

        let (host, port) = match authority.split_once(':') {
            Some((host, port)) => (host, Some(port)),
            None => (authority, None),
        };
        if host.is_empty()
            || host
                .chars()
                .any(|ch| !(ch.is_ascii_alphanumeric() || ch == '.' || ch == '-'))
        {
            return Err(());
        }

        if let Some(port) = port {
            let parsed = port.parse::<u16>().map_err(|_| ())?;
            if parsed == 0 {
                return Err(());
            }
        }

        Ok(Self {
            host: host.to_ascii_lowercase(),
        })
    }
}

fn query_param_name(part: &str) -> &str {
    part.split_once('=').map(|(name, _)| name).unwrap_or(part)
}

fn sanitize_video_id(value: &str) -> Option<String> {
    let video_id: String = value
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || *ch == '-' || *ch == '_')
        .take(128)
        .collect();

    (!video_id.is_empty()).then_some(video_id)
}

async fn supervise_mpv_child(
    mut mpv: Child,
    mut shutdown: oneshot::Receiver<()>,
) -> AnyhowResult<ExitStatus> {
    tokio::select! {
        result = mpv.wait() => {
            result.with_context(|| "failed to wait for mpv child process")
        }
        _ = &mut shutdown => {
            terminate_mpv_child(&mut mpv).await
        }
    }
}

async fn terminate_mpv_child(mpv: &mut Child) -> AnyhowResult<ExitStatus> {
    if let Some(status) = mpv
        .try_wait()
        .with_context(|| "failed to check mpv child process status")?
    {
        return Ok(status);
    }

    mpv.start_kill()
        .with_context(|| "failed to terminate mpv child process")?;
    mpv.wait()
        .await
        .with_context(|| "failed to wait for mpv child process after termination")
}

fn signal_shutdown(shutdown: &mut Option<oneshot::Sender<()>>) {
    if let Some(shutdown) = shutdown.take() {
        let _ = shutdown.send(());
    }
}

async fn await_task<T>(task: JoinHandle<AnyhowResult<T>>, task_name: &str) -> AnyhowResult<T> {
    task_result(task.await, task_name)
}

fn task_result<T>(
    result: std::result::Result<AnyhowResult<T>, tokio::task::JoinError>,
    task_name: &str,
) -> AnyhowResult<T> {
    result.with_context(|| format!("{task_name} task failed"))?
}

fn exit_status_result(status: ExitStatus) -> AnyhowResult<()> {
    if status.success() {
        Ok(())
    } else {
        Err(anyhow::anyhow!("mpv exited with status {status}"))
    }
}

struct Receiver {
    socket_path: PathBuf,
    shutdown: Option<oneshot::Sender<()>>,
    task: Option<JoinHandle<AnyhowResult<ExitStatus>>>,
}

impl Receiver {
    fn start(socket_path: PathBuf) -> AnyhowResult<Self> {
        prepare_mpv_socket_path(&socket_path)?;
        let mpv = TokioCommand::new("mpv")
            .args(mpv_args(&socket_path))
            .stdin(Stdio::null())
            .spawn()
            .with_context(|| "failed to start mpv")?;
        match mpv.id() {
            Some(pid) => info!(pid, "mpv child started"),
            None => info!("mpv child started"),
        }
        let (shutdown, shutdown_rx) = oneshot::channel();
        let task = tokio::spawn(supervise_mpv_child(mpv, shutdown_rx));

        Ok(Self {
            socket_path,
            shutdown: Some(shutdown),
            task: Some(task),
        })
    }

    fn into_parts(
        mut self,
    ) -> (
        PathBuf,
        Option<oneshot::Sender<()>>,
        JoinHandle<AnyhowResult<ExitStatus>>,
    ) {
        let socket_path = std::mem::take(&mut self.socket_path);
        let shutdown = self.shutdown.take();
        let task = self
            .task
            .take()
            .expect("receiver process task should exist");
        (socket_path, shutdown, task)
    }
}

impl Drop for Receiver {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
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

fn prepare_mpv_socket_path(socket_path: &Path) -> AnyhowResult<()> {
    let metadata = match fs::symlink_metadata(socket_path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(error).with_context(|| {
                format!(
                    "failed to inspect mpv IPC socket path {}",
                    socket_path.display()
                )
            });
        }
    };

    if !metadata.file_type().is_socket() {
        return Err(anyhow::anyhow!(
            "mpv IPC socket path {} exists but is not a Unix socket",
            socket_path.display()
        ));
    }

    match UnixStream::connect(socket_path) {
        Ok(_) => Err(anyhow::anyhow!(
            "mpv IPC socket {} is already in use; stop the existing receiver before starting a new one",
            socket_path.display()
        )),
        Err(error) if error.kind() == ErrorKind::ConnectionRefused => {
            fs::remove_file(socket_path).with_context(|| {
                format!(
                    "failed to remove stale mpv IPC socket {}",
                    socket_path.display()
                )
            })?;
            Ok(())
        }
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| {
            format!(
                "failed to connect to existing mpv IPC socket {}",
                socket_path.display()
            )
        }),
    }
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
    use std::{
        io::{Read, Write},
        os::unix::net::UnixListener,
    };

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
            token: Arc::from("0123456789abcdef0123456789abcdef"),
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
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_static("Bearer 0123456789abcdef0123456789abcdef"),
        );

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

    #[tokio::test(flavor = "multi_thread")]
    async fn v1_endpoints_require_bearer_auth_before_body_parsing() {
        let (state, db_path) = test_state();
        let server = TestServer::start(state).await;

        for request in [
            "POST /v1/play HTTP/1.1\r\nHost: localhost\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            "POST /v1/enqueue HTTP/1.1\r\nHost: localhost\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            "POST /v1/control HTTP/1.1\r\nHost: localhost\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            "GET /v1/status HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
            "GET /v1/history HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
        ] {
            let response = server.request(request).await;
            assert!(
                response.starts_with("HTTP/1.1 401"),
                "expected 401 for request:\n{request}\nresponse:\n{response}"
            );
        }

        let invalid = server
            .request(
                "GET /v1/history HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer wrong\r\nConnection: close\r\n\r\n",
            )
            .await;
        assert!(
            invalid.starts_with("HTTP/1.1 401"),
            "expected 401 for invalid token, got:\n{invalid}"
        );

        let query_token = server
            .request(
                "GET /v1/history?token=0123456789abcdef0123456789abcdef HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
            )
            .await;
        assert!(
            query_token.starts_with("HTTP/1.1 401"),
            "expected 401 for query-string token, got:\n{query_token}"
        );

        let _ = fs::remove_file(db_path);
    }

    #[test]
    fn accepts_supported_youtube_urls() {
        assert_eq!(
            validate_supported_url("https://www.youtube.com/watch?v=example")
                .expect("www youtube watch URL should pass"),
            "https://www.youtube.com/watch?v=example"
        );
        assert_eq!(
            validate_supported_url("https://youtube.com/watch?v=example")
                .expect("youtube watch URL should pass"),
            "https://youtube.com/watch?v=example"
        );
        assert_eq!(
            validate_supported_url("https://music.youtube.com/watch?v=example")
                .expect("music youtube watch URL should pass"),
            "https://music.youtube.com/watch?v=example"
        );
        assert_eq!(
            validate_supported_url("https://youtu.be/example").expect("https should pass"),
            "https://youtu.be/example"
        );
    }

    #[test]
    fn strips_playlist_context_from_watch_urls() {
        assert_eq!(
            validate_supported_url("https://www.youtube.com/watch?v=example&list=playlist")
                .expect("watch URL with playlist context should pass"),
            "https://www.youtube.com/watch?v=example"
        );
        assert_eq!(
            validate_supported_url(
                "https://music.youtube.com/watch?v=example&list=playlist&index=2",
            )
            .expect("music watch URL with playlist context should pass"),
            "https://music.youtube.com/watch?v=example&index=2"
        );
    }

    #[test]
    fn rejects_unsafe_url_schemes() {
        let error = validate_supported_url("file:///tmp/audio").expect_err("file URL should fail");
        let javascript =
            validate_supported_url("javascript:alert(1)").expect_err("javascript URL should fail");
        let ftp =
            validate_supported_url("ftp://example.test/audio").expect_err("ftp URL should fail");

        assert_eq!(error.status, StatusCode::BAD_REQUEST);
        assert_eq!(javascript.status, StatusCode::BAD_REQUEST);
        assert_eq!(ftp.status, StatusCode::BAD_REQUEST);
    }

    #[test]
    fn rejects_unsupported_domains() {
        let error =
            validate_supported_url("https://example.test/watch?v=example").expect_err("domain");
        let evil = validate_supported_url("http://evil.example/watch?v=example")
            .expect_err("evil domain should fail");

        assert_eq!(error.status, StatusCode::BAD_REQUEST);
        assert_eq!(evil.status, StatusCode::BAD_REQUEST);
    }

    #[test]
    fn rejects_empty_urls() {
        let error = validate_supported_url("").expect_err("empty URL should fail");

        assert_eq!(error.status, StatusCode::BAD_REQUEST);
    }

    #[test]
    fn rejects_malformed_urls() {
        for url in [
            "not-a-url",
            "https://youtube.com:bad/watch?v=example",
            "https://youtube.com:99999/watch?v=example",
            "https://user@youtube.com/watch?v=example",
            "https://youtube.com/watch?v=with space",
        ] {
            let error = validate_supported_url(url).expect_err("malformed URL should fail");
            assert_eq!(error.status, StatusCode::BAD_REQUEST);
        }
    }

    #[test]
    fn rejects_playlist_urls() {
        let playlist = validate_supported_url("https://www.youtube.com/playlist?list=playlist")
            .expect_err("playlist should fail");
        let music_playlist =
            validate_supported_url("https://music.youtube.com/playlist?list=playlist")
                .expect_err("music playlist should fail");

        assert_eq!(playlist.status, StatusCode::BAD_REQUEST);
        assert_eq!(music_playlist.status, StatusCode::BAD_REQUEST);
    }

    #[test]
    fn rejects_watch_urls_without_video_ids() {
        let error = validate_supported_url("https://www.youtube.com/watch?list=playlist")
            .expect_err("watch URL without video ID should fail");

        assert_eq!(error.status, StatusCode::BAD_REQUEST);
    }

    #[test]
    fn extracts_safe_host_for_rejected_url_logs() {
        assert_eq!(
            safe_host_from_url("ftp://Example.Test/audio"),
            Some("example.test".to_string())
        );
        assert_eq!(
            safe_host_from_url("https://example.test?watch=1"),
            Some("example.test".to_string())
        );
        assert_eq!(safe_host_from_url("https://user@example.test/watch"), None);
        assert_eq!(safe_host_from_url("not-a-url"), None);
    }

    #[test]
    fn extracts_sanitized_youtube_video_ids_for_logs() {
        let watch = ParsedUrl::parse("https://www.youtube.com/watch?v=ynsLjv1AyEg")
            .expect("parse watch URL");
        let short = ParsedUrl::parse("https://youtu.be/ynsLjv1AyEg").expect("parse short URL");
        let odd = ParsedUrl::parse("https://www.youtube.com/watch?v=abc<script>")
            .expect("parse odd watch URL");

        assert_eq!(
            watch.sanitized_youtube_video_id(),
            Some("ynsLjv1AyEg".to_string())
        );
        assert_eq!(
            short.sanitized_youtube_video_id(),
            Some("ynsLjv1AyEg".to_string())
        );
        assert_eq!(
            odd.sanitized_youtube_video_id(),
            Some("abcscript".to_string())
        );
    }

    #[test]
    fn rejects_unsafe_receiver_tokens() {
        assert!(validate_receiver_token("").is_err());
        assert!(validate_receiver_token("change-me").is_err());
        assert!(validate_receiver_token("short-token").is_err());
    }

    #[test]
    fn accepts_receiver_tokens_with_at_least_32_characters() {
        validate_receiver_token("0123456789abcdef0123456789abcdef")
            .expect("32 character token should pass");
    }

    #[test]
    fn removes_stale_mpv_socket_before_startup() {
        let runtime_dir = unique_path("stale-socket");
        fs::create_dir_all(&runtime_dir).expect("create test runtime dir");
        let socket_path = runtime_dir.join("mpv.sock");
        let listener = UnixListener::bind(&socket_path).expect("create stale socket");
        drop(listener);

        prepare_mpv_socket_path(&socket_path).expect("remove stale socket");

        assert!(!socket_path.exists());

        let _ = fs::remove_dir_all(runtime_dir);
    }

    #[test]
    fn rejects_live_mpv_socket_before_startup() {
        let runtime_dir = unique_path("live-socket");
        fs::create_dir_all(&runtime_dir).expect("create test runtime dir");
        let socket_path = runtime_dir.join("mpv.sock");
        let _listener = UnixListener::bind(&socket_path).expect("create live socket");

        let error =
            prepare_mpv_socket_path(&socket_path).expect_err("live socket should block startup");

        assert!(error.to_string().contains("already in use"));
        assert!(socket_path.exists());

        let _ = fs::remove_dir_all(runtime_dir);
    }

    struct TestServer {
        address: SocketAddr,
        task: tokio::task::JoinHandle<()>,
    }

    impl TestServer {
        async fn start(state: AppState) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0")
                .await
                .expect("bind test HTTP server");
            let address = listener.local_addr().expect("read test server address");
            let task = tokio::spawn(async move {
                axum::serve(listener, app(state))
                    .await
                    .expect("run test HTTP server");
            });

            Self { address, task }
        }

        async fn request(&self, request: &str) -> String {
            let address = self.address;
            let request = request.to_string();
            tokio::task::spawn_blocking(move || send_raw_http(address, &request))
                .await
                .expect("join raw HTTP request task")
        }
    }

    impl Drop for TestServer {
        fn drop(&mut self) {
            self.task.abort();
        }
    }

    fn send_raw_http(address: SocketAddr, request: &str) -> String {
        let mut stream = std::net::TcpStream::connect(address).expect("connect test HTTP server");
        stream
            .write_all(request.as_bytes())
            .expect("write raw HTTP request");

        let mut response = String::new();
        stream
            .read_to_string(&mut response)
            .expect("read raw HTTP response");
        response
    }
}
