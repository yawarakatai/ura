use std::{
    fs,
    io::ErrorKind,
    net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket},
    path::PathBuf,
    sync::Arc,
};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::UnixListener,
    sync::oneshot,
};
use tracing::{info, warn};

use crate::{
    config::default_port,
    pairing::{PairingCompletion, PairingManager, PairingStatus},
    playback_runtime::prepare_control_socket_path,
};

pub async fn run_control_socket(
    socket_path: PathBuf,
    bind: SocketAddr,
    pairing: Arc<PairingManager>,
    mut shutdown: oneshot::Receiver<()>,
) -> Result<()> {
    prepare_control_socket_path(&socket_path)?;
    let listener = UnixListener::bind(&socket_path)
        .with_context(|| format!("failed to bind control socket {}", socket_path.display()))?;
    info!(socket_path = %socket_path.display(), "control socket listening");

    loop {
        tokio::select! {
            accepted = listener.accept() => {
                let (stream, _) = accepted.with_context(|| "failed to accept control socket client")?;
                let pairing = Arc::clone(&pairing);
                tokio::spawn(async move {
                    if let Err(error) = handle_control_connection(stream, bind, pairing).await {
                        warn!(error = %error, "control socket request failed");
                    }
                });
            }
            _ = &mut shutdown => {
                pairing.cancel().await;
                break;
            }
        }
    }

    match fs::remove_file(&socket_path) {
        Ok(()) => {}
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => {
            return Err(error).with_context(|| {
                format!("failed to remove control socket {}", socket_path.display())
            });
        }
    }
    Ok(())
}

async fn handle_control_connection(
    mut stream: tokio::net::UnixStream,
    bind: SocketAddr,
    pairing: Arc<PairingManager>,
) -> Result<()> {
    let mut request = String::new();
    stream
        .read_to_string(&mut request)
        .await
        .with_context(|| "failed to read control socket request")?;
    let request: PairControlRequest =
        serde_json::from_str(&request).with_context(|| "invalid control socket request")?;

    let response = match request.command.as_str() {
        "pair_start" => {
            let started = pairing.start().await?;
            info!(expires_in = started.expires_in, "pairing_started");
            PairControlResponse::PairStart {
                code: started.code,
                expires_in: started.expires_in,
                address: display_address_for_bind(bind),
            }
        }
        "pair_cancel" => {
            pairing.cancel().await;
            info!("pairing_cancelled");
            PairControlResponse::Ok { ok: true }
        }
        "pair_status" => PairControlResponse::PairStatus {
            status: pairing.status().await,
            completion: pairing.completion().await,
        },
        other => PairControlResponse::Error {
            error: format!("unsupported control command `{other}`"),
        },
    };

    let body = serde_json::to_vec(&response)?;
    stream
        .write_all(&body)
        .await
        .with_context(|| "failed to write control socket response")?;
    Ok(())
}

#[derive(Debug, Deserialize, Serialize)]
pub struct PairControlRequest {
    pub command: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PairControlResponse {
    PairStart {
        code: String,
        expires_in: u64,
        address: String,
    },
    PairStatus {
        status: PairingStatus,
        completion: PairingCompletion,
    },
    Ok {
        ok: bool,
    },
    Error {
        error: String,
    },
}

pub fn display_address_for_bind(bind: SocketAddr) -> String {
    let ip = match bind.ip() {
        IpAddr::V4(ip) if ip.is_unspecified() => likely_local_ipv4(),
        IpAddr::V6(ip) if ip.is_unspecified() => likely_local_ipv4(),
        ip => ip,
    };
    format_display_address(SocketAddr::new(ip, bind.port()))
}

fn likely_local_ipv4() -> IpAddr {
    UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))
        .and_then(|socket| {
            let _ = socket.connect((Ipv4Addr::new(1, 1, 1, 1), 80));
            socket.local_addr()
        })
        .map(|address| address.ip())
        .ok()
        .filter(|ip| !ip.is_loopback() && ip.is_ipv4())
        .unwrap_or(IpAddr::V4(Ipv4Addr::LOCALHOST))
}

fn format_display_address(address: SocketAddr) -> String {
    if address.port() == default_port() {
        address.ip().to_string()
    } else {
        address.to_string()
    }
}
