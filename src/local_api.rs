use std::{
    net::{Ipv4Addr, SocketAddr},
    sync::Arc,
};

use anyhow::{Context, Result};
use axum::{
    Json, Router,
    extract::{Request, State, rejection::JsonRejection},
    http::{HeaderMap, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use tokio::{net::TcpListener, sync::oneshot};
use tracing::{info, warn};

use crate::{
    client::PairingHttpClient,
    config::default_db_path,
    db::{Database, HistoryEntry},
    mpv::{LoopStatus, MpvStatus},
    node::{DeviceSet, NodeClient},
};

pub const DEFAULT_LOCAL_API_PORT: u16 = 8766;

pub fn default_local_api_address() -> SocketAddr {
    SocketAddr::from((Ipv4Addr::LOCALHOST, DEFAULT_LOCAL_API_PORT))
}

pub async fn run_local_api(receiver_url: String, shutdown: oneshot::Receiver<()>) -> Result<()> {
    let bind = default_local_api_address();
    let state = LocalApiState {
        database: Arc::new(Database::open(default_db_path()?)?),
        receiver_url: Arc::from(receiver_url),
    };
    let app = app(state);
    let listener = TcpListener::bind(bind)
        .await
        .with_context(|| format!("failed to bind local ura control API to {bind}"))?;
    info!(bind = %bind, "local control API listening");

    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = shutdown.await;
        })
        .await
        .with_context(|| "local ura control API failed")
}

fn app(state: LocalApiState) -> Router {
    let auth_state = state.clone();
    Router::new()
        .route("/v1/play", post(play))
        .route("/v1/enqueue", post(enqueue))
        .route("/v1/control", post(control))
        .route("/v1/status", get(status))
        .route("/v1/history", get(history))
        .route("/v1/devices", get(devices))
        .route("/v1/select", post(select))
        .route_layer(middleware::from_fn_with_state(auth_state, require_auth))
        .route("/v1/pair/info", get(pair_info))
        .route("/v1/pair/claim", post(pair_claim))
        .with_state(state)
}

#[derive(Clone)]
struct LocalApiState {
    database: Arc<Database>,
    receiver_url: Arc<str>,
}

#[derive(Debug, Deserialize)]
struct PlayRequest {
    url: String,
}

#[derive(Debug, Deserialize)]
struct ControlRequest {
    command: String,
}

#[derive(Debug, Deserialize)]
struct SelectRequest {
    name: String,
}

#[derive(Debug, Deserialize)]
struct PairClaimRequest {
    code: String,
    device_name: String,
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
struct SelectedResponse {
    selected: String,
}

#[derive(Debug, Serialize)]
struct PairInfoResponse {
    pairing: bool,
    receiver_name: String,
    expires_in: u64,
}

#[derive(Debug, Serialize)]
struct PairClaimResponse {
    protocol_version: u8,
    receiver_name: String,
    token: String,
}

#[derive(Debug, Serialize)]
struct ErrorResponse {
    error: String,
}

#[derive(Debug)]
struct LocalApiError {
    status: StatusCode,
    message: String,
}

impl LocalApiError {
    fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }

    fn internal(error: impl std::fmt::Display) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, error.to_string())
    }
}

impl IntoResponse for LocalApiError {
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
    Json(request): Json<PlayRequest>,
) -> std::result::Result<Json<OkResponse>, LocalApiError> {
    run_node_command(move |client| client.play(&request.url, None)).await?;
    Ok(Json(OkResponse { ok: true }))
}

async fn enqueue(
    Json(request): Json<PlayRequest>,
) -> std::result::Result<Json<OkResponse>, LocalApiError> {
    run_node_command(move |client| client.queue(&request.url, None)).await?;
    Ok(Json(OkResponse { ok: true }))
}

async fn control(
    Json(request): Json<ControlRequest>,
) -> std::result::Result<Json<ControlResponse>, LocalApiError> {
    let command = request.command;
    let loop_status = if command == "loop-status" {
        Some(run_node_command(|client| client.loop_status(None)).await?)
    } else {
        run_node_command(move |client| client.control(&command, None)).await?;
        None
    };
    Ok(Json(ControlResponse {
        ok: true,
        loop_status,
    }))
}

async fn status() -> std::result::Result<Json<MpvStatus>, LocalApiError> {
    Ok(Json(run_node_command(|client| client.status(None)).await?))
}

async fn history() -> std::result::Result<Json<Vec<HistoryEntry>>, LocalApiError> {
    Ok(Json(run_node_command(|client| client.history(None)).await?))
}

async fn devices() -> std::result::Result<Json<DeviceSet>, LocalApiError> {
    Ok(Json(run_node_command(|client| client.devices()).await?))
}

async fn select(
    Json(request): Json<SelectRequest>,
) -> std::result::Result<Json<SelectedResponse>, LocalApiError> {
    if request.name.trim().is_empty() {
        return Err(LocalApiError::new(
            StatusCode::BAD_REQUEST,
            "device name must not be empty",
        ));
    }
    let selected = run_node_command(move |client| client.select(&request.name)).await?;
    Ok(Json(SelectedResponse { selected }))
}

async fn pair_info(
    State(state): State<LocalApiState>,
) -> std::result::Result<Json<PairInfoResponse>, LocalApiError> {
    let receiver_url = state.receiver_url.to_string();
    let info = tokio::task::spawn_blocking(move || PairingHttpClient::new(receiver_url)?.info())
        .await
        .map_err(LocalApiError::internal)?
        .map_err(|error| LocalApiError::new(StatusCode::BAD_GATEWAY, error.to_string()))?;
    Ok(Json(PairInfoResponse {
        pairing: true,
        receiver_name: info.receiver_name,
        expires_in: info.expires_in,
    }))
}

async fn pair_claim(
    State(state): State<LocalApiState>,
    request: std::result::Result<Json<PairClaimRequest>, JsonRejection>,
) -> std::result::Result<Json<PairClaimResponse>, LocalApiError> {
    let Json(request) =
        request.map_err(|_| LocalApiError::new(StatusCode::BAD_REQUEST, "invalid_request"))?;
    if !request
        .code
        .chars()
        .all(|character| character.is_ascii_digit())
        || request.code.len() != 6
    {
        return Err(LocalApiError::new(
            StatusCode::BAD_REQUEST,
            "invalid_pairing_code",
        ));
    }
    if request.device_name.trim().is_empty() {
        return Err(LocalApiError::new(
            StatusCode::BAD_REQUEST,
            "invalid_device_name",
        ));
    }

    let receiver_url = state.receiver_url.to_string();
    let code = request.code;
    let device_name = request.device_name;
    let claim = tokio::task::spawn_blocking(move || {
        PairingHttpClient::new(receiver_url)?.claim(&code, &device_name)
    })
    .await
    .map_err(LocalApiError::internal)?
    .map_err(|error| LocalApiError::new(StatusCode::BAD_GATEWAY, error.to_string()))?;

    Ok(Json(PairClaimResponse {
        protocol_version: claim.protocol_version,
        receiver_name: claim.receiver_name,
        token: claim.token,
    }))
}

async fn require_auth(
    State(state): State<LocalApiState>,
    headers: HeaderMap,
    request: Request,
    next: Next,
) -> std::result::Result<Response, LocalApiError> {
    let Some(token) = bearer_token(&headers) else {
        return Err(LocalApiError::new(
            StatusCode::UNAUTHORIZED,
            "missing bearer token",
        ));
    };

    let database = Arc::clone(&state.database);
    let token = token.to_string();
    let authenticated =
        tokio::task::spawn_blocking(move || database.authenticate_authorized_token(&token))
            .await
            .map_err(LocalApiError::internal)?
            .map_err(LocalApiError::internal)?;

    if !authenticated {
        warn!(reason = "invalid bearer token", "local API auth failure");
        return Err(LocalApiError::new(
            StatusCode::UNAUTHORIZED,
            "invalid bearer token",
        ));
    }

    Ok(next.run(request).await)
}

fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(axum::http::header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
}

async fn run_node_command<T, F>(command: F) -> std::result::Result<T, LocalApiError>
where
    T: Send + 'static,
    F: FnOnce(NodeClient) -> Result<T> + Send + 'static,
{
    tokio::task::spawn_blocking(move || {
        let client = NodeClient::new()?;
        command(client)
    })
    .await
    .map_err(LocalApiError::internal)?
    .map_err(LocalApiError::internal)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_api_is_loopback_only() {
        let address = default_local_api_address();
        assert!(address.ip().is_loopback());
        assert_eq!(address.port(), DEFAULT_LOCAL_API_PORT);
    }

    #[test]
    fn bearer_token_requires_bearer_prefix() {
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            "Bearer secret".parse().expect("header"),
        );
        assert_eq!(bearer_token(&headers), Some("secret"));

        headers.insert(
            axum::http::header::AUTHORIZATION,
            "Basic secret".parse().expect("header"),
        );
        assert_eq!(bearer_token(&headers), None);
    }
}
