use std::{net::SocketAddr, path::PathBuf, sync::Arc};

use anyhow::{Context, Result as AnyhowResult};
use axum::{
    Json, Router,
    extract::{Request, State, rejection::JsonRejection},
    http::{HeaderMap, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use tokio::{net::TcpListener, sync::oneshot, task::JoinHandle};
use tracing::{error, info, warn};

pub use crate::admin_socket::display_address_for_bind;
use crate::admin_socket::run_control_socket;
use crate::config::{
    default_control_socket_path, default_db_path, default_device_name, default_mpv_socket_path,
};
use crate::db::{Database, HistoryEntry, ResumeCheckpoint, authorize_client};
use crate::media_url;
use crate::mpv::{
    LoopMode, MpvClient, MpvEventObserver, SharedPlaybackState, current_playback_checkpoint,
    observed_status, register_playback_request, rollback_playback_request, shared_playback_state,
};
use crate::pairing::{ClaimDecision, PairingManager, PairingStatus};
use crate::playback_runtime::{
    PlaybackRuntime, create_runtime_dir, ensure_program_in_path, exit_status_result,
};

pub async fn run_peer_api(bind: SocketAddr, node_token: String) -> AnyhowResult<()> {
    info!("peer API startup");
    validate_peer_token(&node_token)?;
    ensure_program_in_path("mpv")?;
    ensure_program_in_path("yt-dlp")?;

    let socket_path = default_mpv_socket_path()?;
    let control_socket_path = default_control_socket_path()?;
    info!(socket_path = %socket_path.display(), "mpv IPC socket path");
    create_runtime_dir(&socket_path)?;
    let database = Arc::new(Database::open(default_db_path()?)?);
    let playback_state = shared_playback_state();
    let pairing = Arc::new(PairingManager::new(default_device_name()));

    let playback = PlaybackRuntime::start(socket_path)?;
    let (socket_path, mut mpv_shutdown, mut mpv_task) = playback.into_parts();
    let _mpv_observer = MpvEventObserver::start(
        socket_path.clone(),
        Arc::clone(&database),
        Arc::clone(&playback_state),
    );
    info!(bind = %bind, "peer API bind address");

    let (control_shutdown, control_shutdown_rx) = oneshot::channel();
    let mut control_shutdown = Some(control_shutdown);
    let mut control_task = tokio::spawn(run_control_socket(
        control_socket_path.clone(),
        bind,
        Arc::clone(&pairing),
        control_shutdown_rx,
    ));

    let (api_shutdown, api_shutdown_rx) = oneshot::channel();
    let mut api_shutdown = Some(api_shutdown);
    let mut api_task = tokio::spawn(run_api(
        bind,
        node_token,
        socket_path,
        database,
        playback_state,
        pairing,
        api_shutdown_rx,
    ));

    tokio::select! {
        result = &mut mpv_task => {
            let status = task_result(result, "mpv supervision")?;
            info!(status = %status, "mpv child termination");
            signal_shutdown(&mut api_shutdown);
            signal_shutdown(&mut control_shutdown);
            await_task(api_task, "peer API").await?;
            await_task(control_task, "control socket").await?;
            info!("peer API shutdown");
            exit_status_result(status)
        }
        result = &mut api_task => {
            let api_result = task_result(result, "peer API");
            signal_shutdown(&mut control_shutdown);
            signal_shutdown(&mut mpv_shutdown);
            let control_result = await_task(control_task, "control socket").await;
            let mpv_result = await_task(mpv_task, "mpv supervision").await;
            api_result?;
            control_result?;
            let status = mpv_result?;
            info!(status = %status, "mpv child termination");
            info!("peer API shutdown");
            Ok(())
        }
        result = &mut control_task => {
            let control_result = task_result(result, "control socket");
            signal_shutdown(&mut api_shutdown);
            signal_shutdown(&mut mpv_shutdown);
            let api_result = await_task(api_task, "peer API").await;
            let mpv_result = await_task(mpv_task, "mpv supervision").await;
            control_result?;
            api_result?;
            let status = mpv_result?;
            info!(status = %status, "mpv child termination");
            info!("peer API shutdown");
            Ok(())
        }
        result = tokio::signal::ctrl_c() => {
            info!("shutdown signal received");
            let ctrl_c_result = result.with_context(|| "failed to listen for Ctrl+C");
            signal_shutdown(&mut api_shutdown);
            signal_shutdown(&mut control_shutdown);
            signal_shutdown(&mut mpv_shutdown);
            let mpv_result = await_task(mpv_task, "mpv supervision").await;
            let api_result = await_task(api_task, "peer API").await;
            let control_result = await_task(control_task, "control socket").await;
            ctrl_c_result?;
            let status = mpv_result?;
            api_result?;
            control_result?;
            info!(status = %status, "mpv child termination");
            info!("peer API shutdown");
            Ok(())
        }
    }
}

async fn run_api(
    bind: SocketAddr,
    node_token: String,
    socket_path: PathBuf,
    database: Arc<Database>,
    playback_state: SharedPlaybackState,
    pairing: Arc<PairingManager>,
    shutdown: oneshot::Receiver<()>,
) -> AnyhowResult<()> {
    let state = AppState {
        node_token: Arc::from(node_token),
        socket_path: Arc::new(socket_path),
        database,
        playback_state,
        pairing,
    };
    let app = app(state);
    let listener = TcpListener::bind(bind)
        .await
        .with_context(|| format!("failed to bind peer API to {bind}"))?;
    info!(bind = %bind, "peer API listening");

    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = shutdown.await;
        })
        .await
        .with_context(|| "peer API failed")
}

fn app(state: AppState) -> Router {
    let auth_state = state.clone();
    Router::new()
        .route("/v1/play", post(play))
        .route("/v1/seek", post(seek))
        .route("/v1/control", post(control))
        .route("/v1/status", get(status))
        .route("/v1/history", get(history))
        .route_layer(middleware::from_fn_with_state(auth_state, require_auth))
        .route("/v1/pair/info", get(pair_info))
        .route("/v1/pair/claim", post(pair_claim))
        .with_state(state)
}

#[derive(Clone)]
struct AppState {
    node_token: Arc<str>,
    socket_path: Arc<PathBuf>,
    database: Arc<Database>,
    playback_state: SharedPlaybackState,
    pairing: Arc<PairingManager>,
}

#[derive(Debug, Deserialize)]
struct PlayRequest {
    url: String,
    source: Option<String>,
    #[serde(default, rename = "loop")]
    loop_track: bool,
}

#[derive(Debug, Deserialize)]
struct SeekRequest {
    seconds: f64,
    relative: bool,
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
struct ErrorResponse {
    error: String,
}

#[derive(Debug, Serialize)]
struct PairInfoResponse {
    pairing: bool,
    #[serde(rename = "receiver_name")]
    node_name: String,
    expires_in: u64,
}

#[derive(Debug, Deserialize)]
struct PairClaimRequest {
    code: String,
    device_name: String,
}

#[derive(Debug, Serialize)]
struct PairClaimResponse {
    protocol_version: u8,
    #[serde(rename = "receiver_name")]
    node_name: String,
    token: String,
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
    let loop_mode = if request.loop_track {
        LoopMode::One
    } else {
        LoopMode::Off
    };
    register_playback_request(
        &state.playback_state,
        source_url.clone(),
        play_url.clone(),
        source.clone(),
        None,
        true,
    );

    let result = run_mpv_command(state.clone(), {
        let play_url = play_url.clone();
        move |client| {
            client.set_loop_mode(loop_mode)?;
            client.load_replace(&play_url)
        }
    })
    .await;
    if result.is_err() {
        rollback_playback_request(&state.playback_state, &play_url);
    }
    result?;
    Ok(Json(OkResponse { ok: true }))
}

async fn seek(
    State(state): State<AppState>,
    Json(request): Json<SeekRequest>,
) -> std::result::Result<Json<OkResponse>, AppError> {
    info!(
        request_type = "seek",
        relative = request.relative,
        "HTTP request"
    );
    if !request.seconds.is_finite() || (!request.relative && request.seconds < 0.0) {
        return Err(AppError::new(
            StatusCode::BAD_REQUEST,
            "seek position must be finite and absolute positions must be non-negative",
        ));
    }
    if !has_active_playback(&state.playback_state) {
        return Err(AppError::new(
            StatusCode::BAD_REQUEST,
            "nothing is playing; run `ura resume` first",
        ));
    }

    let checkpoint = seek_checkpoint(&state.playback_state, request.seconds, request.relative);
    run_mpv_command(state.clone(), move |client| {
        client.seek(request.seconds, request.relative)
    })
    .await?;
    if let Some((source_url, position_seconds)) = checkpoint {
        run_db_command(state, move |database| {
            database.save_resume_checkpoint(&source_url, position_seconds)
        })
        .await?;
    }
    Ok(Json(OkResponse { ok: true }))
}

fn seek_checkpoint(
    state: &SharedPlaybackState,
    seconds: f64,
    relative: bool,
) -> Option<(String, f64)> {
    let status = observed_status(state, None);
    let source_url = status.source_url?;
    let position = if relative {
        status.position_seconds? + seconds
    } else {
        seconds
    };
    let position = position.max(0.0);
    let position = status
        .duration_seconds
        .filter(|duration| duration.is_finite() && *duration > 0.0)
        .map_or(position, |duration| position.min(duration));
    Some((source_url, position))
}

async fn control(
    State(state): State<AppState>,
    Json(request): Json<ControlRequest>,
) -> std::result::Result<Json<OkResponse>, AppError> {
    let command = request.command;
    info!(request_type = "control", command = %command, "HTTP request");

    match command.as_str() {
        "pause" => pause_playback(state).await?,
        "resume" => resume_playback(state).await?,
        "stop" => stop_playback(state).await?,
        other => {
            return Err(AppError::new(
                StatusCode::BAD_REQUEST,
                format!("unsupported control command `{other}`"),
            ));
        }
    }

    Ok(Json(OkResponse { ok: true }))
}

async fn pause_playback(state: AppState) -> std::result::Result<(), AppError> {
    let status = observed_status(&state.playback_state, None);
    if !status_has_active_playback(&status) || status.pause == Some(true) {
        return Ok(());
    }

    save_current_checkpoint(&state).await?;
    run_mpv_command(state, |client| client.pause()).await
}

async fn stop_playback(state: AppState) -> std::result::Result<(), AppError> {
    if !has_active_playback(&state.playback_state) {
        return Ok(());
    }

    save_current_checkpoint(&state).await?;
    run_mpv_command(state, |client| client.stop()).await
}

async fn resume_playback(state: AppState) -> std::result::Result<(), AppError> {
    let status = observed_status(&state.playback_state, None);
    if status_has_active_playback(&status) {
        if status.pause == Some(true) {
            run_mpv_command(state, |client| client.resume()).await?;
        }
        return Ok(());
    }

    let checkpoint = run_db_command(state.clone(), |database| database.resume_checkpoint())
        .await?
        .ok_or_else(|| AppError::new(StatusCode::BAD_REQUEST, "nothing to resume"))?;
    let play_url = validate_supported_url(&checkpoint.source_url)?;
    register_playback_request(
        &state.playback_state,
        checkpoint.source_url,
        play_url.clone(),
        Some("resume".to_string()),
        Some(checkpoint.position_seconds),
        false,
    );

    let result = run_mpv_command(state.clone(), {
        let play_url = play_url.clone();
        move |client| {
            client.set_loop_mode(LoopMode::Off)?;
            client.load_replace(&play_url)
        }
    })
    .await;
    if result.is_err() {
        rollback_playback_request(&state.playback_state, &play_url);
    }
    result
}

async fn save_current_checkpoint(state: &AppState) -> std::result::Result<(), AppError> {
    let Some((source_url, position_seconds)) = current_playback_checkpoint(&state.playback_state)
    else {
        return Ok(());
    };
    run_db_command(state.clone(), move |database| {
        database.save_resume_checkpoint(&source_url, position_seconds)
    })
    .await
}

fn has_active_playback(state: &SharedPlaybackState) -> bool {
    status_has_active_playback(&observed_status(state, None))
}

fn status_has_active_playback(status: &crate::mpv::MpvStatus) -> bool {
    status.idle_active != Some(true)
        && (status.path.is_some() || status.source_url.is_some() || status.media_title.is_some())
}

fn apply_resume_checkpoint(status: &mut crate::mpv::MpvStatus, checkpoint: ResumeCheckpoint) {
    status.pause = None;
    status.idle_active = Some(true);
    status.path = None;
    status.media_title = checkpoint.title.clone();
    status.title = checkpoint.title;
    status.artist = None;
    status.uploader = checkpoint.uploader;
    status.album = None;
    status.duration_seconds = checkpoint.duration_seconds;
    status.position_seconds = Some(checkpoint.position_seconds);
    status.source_url = Some(checkpoint.source_url);
    status.playback_path = None;
    status.loop_status = None;
    status.resume_available = true;
}

async fn status(
    State(state): State<AppState>,
) -> std::result::Result<Json<crate::mpv::MpvStatus>, AppError> {
    info!(request_type = "status", "HTTP request");
    let loop_status = run_mpv_command(state.clone(), |client| client.loop_status())
        .await
        .ok();
    let mut status = observed_status(&state.playback_state, loop_status);
    if !status_has_active_playback(&status)
        && let Some(checkpoint) =
            run_db_command(state, |database| database.resume_checkpoint()).await?
    {
        apply_resume_checkpoint(&mut status, checkpoint);
    }
    Ok(Json(status))
}

async fn history(
    State(state): State<AppState>,
) -> std::result::Result<Json<Vec<HistoryEntry>>, AppError> {
    info!(request_type = "history", "HTTP request");
    let history = run_db_command(state, |database| database.history()).await?;
    Ok(Json(history))
}

async fn pair_info(
    State(state): State<AppState>,
) -> std::result::Result<Json<PairInfoResponse>, AppError> {
    match state.pairing.status().await {
        PairingStatus::Active {
            node_name,
            expires_in,
            ..
        } => Ok(Json(PairInfoResponse {
            pairing: true,
            node_name,
            expires_in,
        })),
        PairingStatus::Inactive => Err(AppError::new(StatusCode::FORBIDDEN, "pairing_not_active")),
    }
}

async fn pair_claim(
    State(state): State<AppState>,
    request: Result<Json<PairClaimRequest>, JsonRejection>,
) -> std::result::Result<Json<PairClaimResponse>, AppError> {
    let Json(request) =
        request.map_err(|_| AppError::new(StatusCode::BAD_REQUEST, "invalid_request"))?;
    if request.device_name.trim().is_empty() {
        return Err(AppError::new(
            StatusCode::BAD_REQUEST,
            "invalid_device_name",
        ));
    }

    match state.pairing.begin_claim(&request.code).await {
        ClaimDecision::Accepted => {}
        ClaimDecision::Inactive => {
            return Err(AppError::new(StatusCode::FORBIDDEN, "pairing_not_active"));
        }
        ClaimDecision::InvalidRequest => {
            return Err(AppError::new(
                StatusCode::BAD_REQUEST,
                "invalid_pairing_code",
            ));
        }
        ClaimDecision::WrongCode { remaining_attempts } => {
            warn!(remaining_attempts, "pairing_attempt_failed");
            return Err(AppError::new(
                StatusCode::UNAUTHORIZED,
                "invalid_pairing_code",
            ));
        }
        ClaimDecision::AttemptsExhausted => {
            warn!("pairing_attempts_exhausted");
            return Err(AppError::new(
                StatusCode::TOO_MANY_REQUESTS,
                "pairing_attempts_exhausted",
            ));
        }
    }

    let node_name = match state.pairing.status().await {
        PairingStatus::Active { node_name, .. } => node_name,
        PairingStatus::Inactive => {
            state.pairing.fail_claim().await;
            return Err(AppError::new(StatusCode::FORBIDDEN, "pairing_not_active"));
        }
    };
    let device_name = request.device_name;
    let database = Arc::clone(&state.database);
    let device_name_for_db = device_name.clone();
    let token = tokio::task::spawn_blocking(move || {
        if database
            .authorized_clients()?
            .iter()
            .any(|device| device.name == device_name_for_db)
        {
            anyhow::bail!("duplicate authorized client");
        }
        authorize_client(&database, &device_name_for_db)
    })
    .await
    .map_err(|error| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?
    .map_err(|error| {
        if error.to_string().contains("duplicate authorized client")
            || error.to_string().contains("UNIQUE constraint")
        {
            AppError::new(StatusCode::CONFLICT, "device_name_exists")
        } else {
            AppError::new(StatusCode::INTERNAL_SERVER_ERROR, error.to_string())
        }
    });

    let token = match token {
        Ok(token) => token,
        Err(error) => {
            state.pairing.fail_claim().await;
            return Err(error);
        }
    };

    info!(device_name = %device_name, "pairing_succeeded");
    state.pairing.complete_claim(device_name).await;
    Ok(Json(PairClaimResponse {
        protocol_version: 1,
        node_name,
        token,
    }))
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
    let Some(actual) = headers.get(axum::http::header::AUTHORIZATION) else {
        warn!(reason = "missing bearer token", "auth failure");
        return Err(AppError::new(
            StatusCode::UNAUTHORIZED,
            "missing bearer token",
        ));
    };
    let Some(actual) = actual
        .to_str()
        .ok()
        .and_then(|value| value.strip_prefix("Bearer "))
    else {
        warn!(reason = "invalid bearer token", "auth failure");
        return Err(AppError::new(
            StatusCode::UNAUTHORIZED,
            "invalid bearer token",
        ));
    };

    if state
        .database
        .authenticate_authorized_token(actual)
        .map_err(|error| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?
    {
        return Ok(());
    }

    if crate::security::constant_time_eq(actual.as_bytes(), state.node_token.as_bytes()) {
        Ok(())
    } else {
        warn!(reason = "invalid bearer token", "auth failure");
        Err(AppError::new(
            StatusCode::UNAUTHORIZED,
            "invalid bearer token",
        ))
    }
}

pub fn validate_peer_token(token: &str) -> AnyhowResult<()> {
    let token = token.trim();
    if token.is_empty() {
        return Err(anyhow::anyhow!("peer API token must not be empty"));
    }
    if token == "change-me" {
        return Err(anyhow::anyhow!(
            "peer API token must be changed before starting the node"
        ));
    }
    if token.len() < 32 {
        return Err(anyhow::anyhow!(
            "peer API token must be at least 32 characters"
        ));
    }

    Ok(())
}

fn validate_supported_url(url: &str) -> std::result::Result<String, AppError> {
    media_url::validate(url)
        .map_err(|error| AppError::new(StatusCode::BAD_REQUEST, error.to_string()))
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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{HeaderMap, HeaderValue, header::AUTHORIZATION};
    use std::{
        fs,
        io::{Read, Write},
        os::unix::net::UnixListener,
        path::Path,
    };

    use crate::playback_runtime::{mpv_args, prepare_mpv_socket_path};

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
            node_token: Arc::from("0123456789abcdef0123456789abcdef"),
            socket_path: Arc::new(PathBuf::from("/tmp/ura.sock")),
            database: Arc::new(Database::open(db_path.clone()).expect("open test database")),
            playback_state: shared_playback_state(),
            pairing: Arc::new(PairingManager::new("kamo".to_string())),
        };
        (state, db_path)
    }

    #[test]
    fn seek_checkpoint_tracks_relative_and_clamped_positions() {
        let state = shared_playback_state();
        {
            let mut state = state.lock().expect("state lock");
            state.current_source_url = Some("https://youtu.be/example".to_string());
            state.position_seconds = Some(40.0);
            state.duration_seconds = Some(60.0);
        }

        assert_eq!(
            seek_checkpoint(&state, 10.0, true),
            Some(("https://youtu.be/example".to_string(), 50.0))
        );
        assert_eq!(
            seek_checkpoint(&state, -100.0, true),
            Some(("https://youtu.be/example".to_string(), 0.0))
        );
        assert_eq!(
            seek_checkpoint(&state, 90.0, false),
            Some(("https://youtu.be/example".to_string(), 60.0))
        );
    }

    #[tokio::test]
    async fn seek_rejects_idle_playback_without_contacting_mpv() {
        let (state, db_path) = test_state();

        let error = seek(
            State(state),
            Json(SeekRequest {
                seconds: 30.0,
                relative: true,
            }),
        )
        .await
        .expect_err("idle seek should fail");

        assert_eq!(error.status, StatusCode::BAD_REQUEST);
        assert!(error.message.contains("nothing is playing"));
        let _ = fs::remove_file(db_path);
    }

    #[tokio::test]
    async fn resume_rejects_when_no_checkpoint_exists() {
        let (state, db_path) = test_state();

        let error = resume_playback(state)
            .await
            .expect_err("missing checkpoint should fail");

        assert_eq!(error.status, StatusCode::BAD_REQUEST);
        assert_eq!(error.message, "nothing to resume");
        let _ = fs::remove_file(db_path);
    }

    #[tokio::test]
    async fn status_exposes_a_saved_resume_checkpoint() {
        let (state, db_path) = test_state();
        state
            .database
            .record_play(
                "https://youtu.be/example",
                "https://youtu.be/example",
                Some("cli"),
            )
            .expect("record play");
        state
            .database
            .update_track_metadata(
                "https://youtu.be/example",
                &crate::mpv::MediaMetadata {
                    title: Some("Example song".to_string()),
                    uploader: Some("Example artist".to_string()),
                    duration_seconds: Some(300.0),
                    ..crate::mpv::MediaMetadata::default()
                },
            )
            .expect("update metadata");
        state
            .database
            .save_resume_checkpoint("https://youtu.be/example", 42.0)
            .expect("save checkpoint");

        let Json(status) = status(State(state)).await.expect("read status");

        assert!(status.resume_available);
        assert_eq!(status.title.as_deref(), Some("Example song"));
        assert_eq!(status.uploader.as_deref(), Some("Example artist"));
        assert_eq!(status.position_seconds, Some(42.0));
        assert_eq!(status.duration_seconds, Some(300.0));
        let _ = fs::remove_file(db_path);
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

    #[test]
    fn generated_authorized_token_authenticates() {
        let (state, db_path) = test_state();
        state
            .database
            .authorize_client("desuwa", "abcdef0123456789abcdef0123456789")
            .expect("authorize device");
        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_static("Bearer abcdef0123456789abcdef0123456789"),
        );

        authorize(&headers, &state).expect("authorized token should pass");

        let _ = fs::remove_file(db_path);
    }

    #[test]
    fn revoked_authorized_token_is_rejected() {
        let (state, db_path) = test_state();
        state
            .database
            .authorize_client("desuwa", "abcdef0123456789abcdef0123456789")
            .expect("authorize device");
        state
            .database
            .revoke_authorized_client("desuwa")
            .expect("revoke device");
        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_static("Bearer abcdef0123456789abcdef0123456789"),
        );

        let error = authorize(&headers, &state).expect_err("revoked token should fail");

        assert_eq!(error.status, StatusCode::UNAUTHORIZED);
        let _ = fs::remove_file(db_path);
    }

    #[test]
    fn second_authorized_device_authenticates_independently() {
        let (state, db_path) = test_state();
        state
            .database
            .authorize_client("desuwa", "abcdef0123456789abcdef0123456789")
            .expect("authorize first");
        state
            .database
            .authorize_client("desktop-client", "fedcba9876543210fedcba9876543210")
            .expect("authorize second");
        state
            .database
            .revoke_authorized_client("desuwa")
            .expect("revoke first");
        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_static("Bearer fedcba9876543210fedcba9876543210"),
        );

        authorize(&headers, &state).expect("second token should pass");

        let _ = fs::remove_file(db_path);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn v1_endpoints_require_bearer_auth_before_body_parsing() {
        let (state, db_path) = test_state();
        let server = TestServer::start(state).await;

        for request in [
            "POST /v1/play HTTP/1.1\r\nHost: localhost\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            "POST /v1/seek HTTP/1.1\r\nHost: localhost\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
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

    #[tokio::test(flavor = "multi_thread")]
    async fn pair_info_is_forbidden_when_inactive() {
        let (state, db_path) = test_state();
        let server = TestServer::start(state).await;

        let response = server
            .request("GET /v1/pair/info HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
            .await;

        assert!(response.starts_with("HTTP/1.1 403"), "{response}");
        assert!(response.contains("pairing_not_active"));
        let _ = fs::remove_file(db_path);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn pair_info_returns_minimal_active_state() {
        let (state, db_path) = test_state();
        state
            .pairing
            .start_for_test("482913", std::time::Duration::from_secs(120))
            .await;
        let server = TestServer::start(state).await;

        let response = server
            .request("GET /v1/pair/info HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
            .await;

        assert!(response.starts_with("HTTP/1.1 200"), "{response}");
        assert!(response.contains(r#""receiver_name":"kamo""#));
        assert!(response.contains(r#""expires_in":"#));
        assert!(!response.contains("482913"));
        assert!(!response.contains("token"));
        let _ = fs::remove_file(db_path);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn pair_claim_errors_are_consistent() {
        let (state, db_path) = test_state();
        state
            .pairing
            .start_for_test("482913", std::time::Duration::from_secs(120))
            .await;
        let server = TestServer::start(state).await;

        let malformed = server
            .request("POST /v1/pair/claim HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: 1\r\nConnection: close\r\n\r\n{")
            .await;
        assert!(malformed.starts_with("HTTP/1.1 400"), "{malformed}");
        assert!(malformed.contains("invalid_request"));

        let invalid_code = server
            .request("POST /v1/pair/claim HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: 37\r\nConnection: close\r\n\r\n{\"code\":\"bad\",\"device_name\":\"desuwa\"}")
            .await;
        assert!(invalid_code.starts_with("HTTP/1.1 400"), "{invalid_code}");

        let wrong = server
            .request("POST /v1/pair/claim HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: 40\r\nConnection: close\r\n\r\n{\"code\":\"111111\",\"device_name\":\"desuwa\"}")
            .await;
        assert!(wrong.starts_with("HTTP/1.1 401"), "{wrong}");

        let _ = fs::remove_file(db_path);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn correct_pair_claim_returns_token_and_closes_pairing() {
        let (state, db_path) = test_state();
        state
            .pairing
            .start_for_test("482913", std::time::Duration::from_secs(120))
            .await;
        let server = TestServer::start(state.clone()).await;

        let response = server
            .request("POST /v1/pair/claim HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: 40\r\nConnection: close\r\n\r\n{\"code\":\"482913\",\"device_name\":\"desuwa\"}")
            .await;

        assert!(response.starts_with("HTTP/1.1 200"), "{response}");
        let body = response_body(&response);
        let claim: PairClaimResponseForTest = serde_json::from_str(body).expect("claim JSON");
        assert_eq!(claim.protocol_version, 1);
        assert_eq!(claim.receiver_name, "kamo");
        assert!(claim.token.len() >= 64);
        assert_eq!(state.pairing.status().await, PairingStatus::Inactive);

        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {}", claim.token)).expect("auth header"),
        );
        authorize(&headers, &state).expect("paired token should authenticate");

        let second = server
            .request("POST /v1/pair/claim HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: 40\r\nConnection: close\r\n\r\n{\"code\":\"482913\",\"device_name\":\"second\"}")
            .await;
        assert!(second.starts_with("HTTP/1.1 403"), "{second}");

        let _ = fs::remove_file(db_path);
    }

    #[derive(Debug, Deserialize)]
    struct PairClaimResponseForTest {
        protocol_version: u8,
        receiver_name: String,
        token: String,
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
    fn rejects_unsafe_receiver_tokens() {
        assert!(validate_peer_token("").is_err());
        assert!(validate_peer_token("change-me").is_err());
        assert!(validate_peer_token("short-token").is_err());
    }

    #[test]
    fn accepts_receiver_tokens_with_at_least_32_characters() {
        validate_peer_token("0123456789abcdef0123456789abcdef")
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

    fn response_body(response: &str) -> &str {
        response
            .split_once("\r\n\r\n")
            .map(|(_, body)| body)
            .expect("HTTP response has body separator")
    }
}
