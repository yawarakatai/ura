use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    net::{IpAddr, Ipv4Addr, Ipv6Addr, Shutdown, SocketAddr},
    os::unix::{
        fs::{FileTypeExt, OpenOptionsExt, PermissionsExt},
        net::UnixStream,
    },
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
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
    client::HttpClient,
    config::{DeviceConfig, RemoteDevice, config_path, default_device_name, generate_token},
    db::HistoryEntry,
    mpv::{LoopStatus, MpvStatus},
    receiver::run_serve,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Destination {
    SelfNode { name: String },
    Peer(RemoteDevice),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceKind {
    ThisDevice,
    Peer,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceSummary {
    pub name: String,
    pub kind: DeviceKind,
    pub address: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceSet {
    pub selected: String,
    pub devices: Vec<DeviceSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
enum NodeRequest {
    Ping,
    Play { url: String, to: Option<String> },
    Queue { url: String, to: Option<String> },
    Control { action: String, to: Option<String> },
    LoopStatus { to: Option<String> },
    Status { to: Option<String> },
    History { to: Option<String> },
    Devices,
    Select { name: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum NodeResponse {
    Ok,
    LoopStatus { status: LoopStatus },
    Status { status: MpvStatus },
    History { entries: Vec<HistoryEntry> },
    Devices { devices: DeviceSet },
    Selected { name: String },
    Error { error: String },
}

#[derive(Debug, Clone)]
struct NodeRuntime {
    local_url: String,
    local_token: String,
    config_path: Option<PathBuf>,
}

pub struct NodeClient {
    socket_path: PathBuf,
}

impl NodeClient {
    pub fn new() -> Result<Self> {
        Ok(Self {
            socket_path: default_node_socket_path()?,
        })
    }

    pub fn is_available() -> bool {
        Self::new()
            .and_then(|client| client.expect_ok(NodeRequest::Ping))
            .is_ok()
    }

    pub fn play(&self, url: &str, to: Option<&str>) -> Result<()> {
        self.expect_ok(NodeRequest::Play {
            url: url.to_string(),
            to: to.map(str::to_string),
        })
    }

    pub fn queue(&self, url: &str, to: Option<&str>) -> Result<()> {
        self.expect_ok(NodeRequest::Queue {
            url: url.to_string(),
            to: to.map(str::to_string),
        })
    }

    pub fn control(&self, action: &str, to: Option<&str>) -> Result<()> {
        self.expect_ok(NodeRequest::Control {
            action: action.to_string(),
            to: to.map(str::to_string),
        })
    }

    pub fn loop_status(&self, to: Option<&str>) -> Result<LoopStatus> {
        match self.request(NodeRequest::LoopStatus {
            to: to.map(str::to_string),
        })? {
            NodeResponse::LoopStatus { status } => Ok(status),
            other => unexpected_response(other),
        }
    }

    pub fn status(&self, to: Option<&str>) -> Result<MpvStatus> {
        match self.request(NodeRequest::Status {
            to: to.map(str::to_string),
        })? {
            NodeResponse::Status { status } => Ok(status),
            other => unexpected_response(other),
        }
    }

    pub fn history(&self, to: Option<&str>) -> Result<Vec<HistoryEntry>> {
        match self.request(NodeRequest::History {
            to: to.map(str::to_string),
        })? {
            NodeResponse::History { entries } => Ok(entries),
            other => unexpected_response(other),
        }
    }

    pub fn devices(&self) -> Result<DeviceSet> {
        match self.request(NodeRequest::Devices)? {
            NodeResponse::Devices { devices } => Ok(devices),
            other => unexpected_response(other),
        }
    }

    pub fn select(&self, name: &str) -> Result<String> {
        match self.request(NodeRequest::Select {
            name: name.to_string(),
        })? {
            NodeResponse::Selected { name } => Ok(name),
            other => unexpected_response(other),
        }
    }

    fn expect_ok(&self, request: NodeRequest) -> Result<()> {
        match self.request(request)? {
            NodeResponse::Ok => Ok(()),
            other => unexpected_response(other),
        }
    }

    fn request(&self, request: NodeRequest) -> Result<NodeResponse> {
        let mut stream = UnixStream::connect(&self.socket_path).with_context(|| {
            format!(
                "ura node is not running or node socket is unavailable at {}",
                self.socket_path.display()
            )
        })?;
        let body = serde_json::to_vec(&request)?;
        stream.write_all(&body)?;
        stream.shutdown(Shutdown::Write)?;

        let mut response = String::new();
        stream.read_to_string(&mut response)?;
        let response: NodeResponse = serde_json::from_str(&response)
            .with_context(|| "failed to parse local ura node response")?;
        match response {
            NodeResponse::Error { error } => anyhow::bail!("{error}"),
            other => Ok(other),
        }
    }
}

fn unexpected_response<T>(response: NodeResponse) -> Result<T> {
    anyhow::bail!("ura node returned an unexpected response: {response:?}")
}

pub async fn run_node(
    bind: SocketAddr,
    token: Option<String>,
    config_path: Option<PathBuf>,
) -> Result<()> {
    let local_token = token.unwrap_or(generate_token()?);
    let local_url = local_url_for_bind(bind);
    let runtime = Arc::new(NodeRuntime {
        local_url,
        local_token: local_token.clone(),
        config_path,
    });

    info!("node startup");
    let mut receiver_task = tokio::spawn(run_serve(bind, Some(local_token)));
    wait_for_receiver(bind, &mut receiver_task).await?;

    let (node_shutdown, node_shutdown_rx) = oneshot::channel();
    let mut node_shutdown = Some(node_shutdown);
    let mut node_task = tokio::spawn(run_node_socket(
        default_node_socket_path()?,
        runtime,
        node_shutdown_rx,
    ));

    tokio::select! {
        result = &mut receiver_task => {
            if let Some(shutdown) = node_shutdown.take() {
                let _ = shutdown.send(());
            }
            flatten_task_result(result, "receiver")?;
            let _ = node_task.await;
            Ok(())
        }
        result = &mut node_task => {
            flatten_task_result(result, "node control socket")?;
            receiver_task.abort();
            Ok(())
        }
    }
}

async fn wait_for_receiver(
    bind: SocketAddr,
    receiver_task: &mut tokio::task::JoinHandle<Result<()>>,
) -> Result<()> {
    let address = local_address_for_bind(bind);
    for _ in 0..100 {
        if receiver_task.is_finished() {
            let result = (&mut *receiver_task)
                .await
                .map_err(|error| anyhow::anyhow!("receiver task failed: {error}"))?;
            result?;
            anyhow::bail!("receiver exited during node startup");
        }
        if tokio::net::TcpStream::connect(address).await.is_ok() {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    anyhow::bail!("timed out waiting for the local ura receiver at {address}")
}

fn flatten_task_result(
    result: std::result::Result<Result<()>, tokio::task::JoinError>,
    label: &str,
) -> Result<()> {
    result.map_err(|error| anyhow::anyhow!("{label} task failed: {error}"))?
}

async fn run_node_socket(
    socket_path: PathBuf,
    runtime: Arc<NodeRuntime>,
    mut shutdown: oneshot::Receiver<()>,
) -> Result<()> {
    prepare_socket_path(&socket_path)?;
    let listener = UnixListener::bind(&socket_path)
        .with_context(|| format!("failed to bind node socket {}", socket_path.display()))?;
    fs::set_permissions(&socket_path, fs::Permissions::from_mode(0o600))?;
    info!(socket_path = %socket_path.display(), "node control socket listening");

    loop {
        tokio::select! {
            accepted = listener.accept() => {
                let (stream, _) = accepted.with_context(|| "failed to accept node socket client")?;
                let runtime = Arc::clone(&runtime);
                tokio::spawn(async move {
                    if let Err(error) = handle_node_connection(stream, runtime).await {
                        warn!(error = %error, "node control request failed");
                    }
                });
            }
            _ = &mut shutdown => break,
        }
    }

    remove_socket_if_present(&socket_path)?;
    Ok(())
}

async fn handle_node_connection(
    mut stream: tokio::net::UnixStream,
    runtime: Arc<NodeRuntime>,
) -> Result<()> {
    let mut request = String::new();
    stream
        .read_to_string(&mut request)
        .await
        .with_context(|| "failed to read node control request")?;
    let request: NodeRequest =
        serde_json::from_str(&request).with_context(|| "invalid node control request")?;

    let response = match dispatch(request, &runtime).await {
        Ok(response) => response,
        Err(error) => NodeResponse::Error {
            error: format!("{error:#}"),
        },
    };
    let body = serde_json::to_vec(&response)?;
    stream
        .write_all(&body)
        .await
        .with_context(|| "failed to write node control response")?;
    Ok(())
}

async fn dispatch(request: NodeRequest, runtime: &NodeRuntime) -> Result<NodeResponse> {
    match request {
        NodeRequest::Ping => Ok(NodeResponse::Ok),
        NodeRequest::Play { url, to } => {
            route(runtime, to, move |client| client.play(&url)).await?;
            Ok(NodeResponse::Ok)
        }
        NodeRequest::Queue { url, to } => {
            route(runtime, to, move |client| client.queue(&url)).await?;
            Ok(NodeResponse::Ok)
        }
        NodeRequest::Control { action, to } => {
            route(runtime, to, move |client| client.control(&action)).await?;
            Ok(NodeResponse::Ok)
        }
        NodeRequest::LoopStatus { to } => {
            let status = route(runtime, to, |client| client.loop_status()).await?;
            Ok(NodeResponse::LoopStatus { status })
        }
        NodeRequest::Status { to } => {
            let status = route(runtime, to, |client| client.status()).await?;
            Ok(NodeResponse::Status { status })
        }
        NodeRequest::History { to } => {
            let entries = route(runtime, to, |client| client.history()).await?;
            Ok(NodeResponse::History { entries })
        }
        NodeRequest::Devices => Ok(NodeResponse::Devices {
            devices: device_set(runtime.config_path.as_deref())?,
        }),
        NodeRequest::Select { name } => {
            let name = select_device(runtime.config_path.as_deref(), &name)?;
            Ok(NodeResponse::Selected { name })
        }
    }
}

async fn route<T, F>(runtime: &NodeRuntime, to: Option<String>, operation: F) -> Result<T>
where
    T: Send + 'static,
    F: FnOnce(HttpClient) -> Result<T> + Send + 'static,
{
    let destination = resolve_destination(runtime.config_path.as_deref(), to.as_deref())?;
    let (url, token) = match destination {
        Destination::SelfNode { .. } => (runtime.local_url.clone(), runtime.local_token.clone()),
        Destination::Peer(peer) => (peer.url, peer.token),
    };
    tokio::task::spawn_blocking(move || {
        let client = HttpClient::new(url, token)?;
        operation(client)
    })
    .await
    .map_err(|error| anyhow::anyhow!("destination request task failed: {error}"))?
}

pub fn resolve_destination(config_path: Option<&Path>, to: Option<&str>) -> Result<Destination> {
    let config = DeviceConfig::load(config_path)?;
    let local_name = config
        .local_name
        .clone()
        .unwrap_or_else(default_device_name);

    if let Some(name) = to {
        if is_local_name(name, &local_name) {
            return Ok(Destination::SelfNode { name: local_name });
        }
        return config
            .devices
            .iter()
            .find(|device| device.name == name)
            .cloned()
            .or_else(|| {
                config
                    .legacy_device
                    .as_ref()
                    .filter(|device| device.name == name)
                    .cloned()
            })
            .map(Destination::Peer)
            .ok_or_else(|| anyhow::anyhow!("unknown device `{name}`"));
    }

    if let Some(selected) = &config.selected_device {
        let peer = config
            .devices
            .iter()
            .find(|device| &device.name == selected)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("selected device `{selected}` does not exist"))?;
        return Ok(Destination::Peer(peer));
    }

    if let Some(legacy) = &config.legacy_device {
        return Ok(Destination::Peer(legacy.clone()));
    }

    Ok(Destination::SelfNode { name: local_name })
}

pub fn device_set(config_path: Option<&Path>) -> Result<DeviceSet> {
    let config = DeviceConfig::load(config_path)?;
    let local_name = config
        .local_name
        .clone()
        .unwrap_or_else(default_device_name);
    let selected = config
        .selected_device
        .clone()
        .or_else(|| config.legacy_device.as_ref().map(|device| device.name.clone()))
        .unwrap_or_else(|| local_name.clone());

    let mut devices = Vec::with_capacity(config.devices.len() + 2);
    devices.push(DeviceSummary {
        name: local_name,
        kind: DeviceKind::ThisDevice,
        address: None,
    });
    devices.extend(config.devices.iter().map(|device| DeviceSummary {
        name: device.name.clone(),
        kind: DeviceKind::Peer,
        address: Some(device.url.clone()),
    }));
    if config.devices.is_empty()
        && let Some(legacy) = &config.legacy_device
    {
        devices.push(DeviceSummary {
            name: legacy.name.clone(),
            kind: DeviceKind::Peer,
            address: Some(legacy.url.clone()),
        });
    }

    Ok(DeviceSet { selected, devices })
}

pub fn select_device(config_path_override: Option<&Path>, name: &str) -> Result<String> {
    let config = DeviceConfig::load(config_path_override)?;
    let devices = device_set(config_path_override)?;
    let local_name = devices
        .devices
        .iter()
        .find(|device| device.kind == DeviceKind::ThisDevice)
        .map(|device| device.name.clone())
        .ok_or_else(|| anyhow::anyhow!("local node is missing"))?;

    let selected_remote = if is_local_name(name, &local_name) {
        None
    } else {
        let device = devices
            .devices
            .iter()
            .find(|device| device.kind == DeviceKind::Peer && device.name == name)
            .ok_or_else(|| anyhow::anyhow!("unknown device `{name}`"))?;
        Some(device.name.clone())
    };

    let path = config_path(config_path_override)?;
    if selected_remote.is_none() && !path.exists() {
        return Ok(local_name);
    }

    let mut root = if path.exists() {
        let contents = fs::read_to_string(&path)
            .with_context(|| format!("failed to read config file {}", path.display()))?;
        toml::from_str::<toml::Value>(&contents)
            .with_context(|| format!("failed to parse config file {}", path.display()))?
    } else {
        toml::Value::Table(Default::default())
    };
    let table = root
        .as_table_mut()
        .ok_or_else(|| anyhow::anyhow!("config root must be a TOML table"))?;

    if config.devices.is_empty()
        && let Some(legacy) = &config.legacy_device
    {
        let mut peer = toml::map::Map::new();
        peer.insert("name".to_string(), toml::Value::String(legacy.name.clone()));
        peer.insert("url".to_string(), toml::Value::String(legacy.url.clone()));
        peer.insert("token".to_string(), toml::Value::String(legacy.token.clone()));
        table.insert(
            "devices".to_string(),
            toml::Value::Array(vec![toml::Value::Table(peer)]),
        );
    }

    match &selected_remote {
        Some(name) => {
            table.insert(
                "selected_device".to_string(),
                toml::Value::String(name.clone()),
            );
        }
        None => {
            table.remove("selected_device");
        }
    }

    write_toml_atomic(&path, &root)?;
    Ok(selected_remote.unwrap_or(local_name))
}

fn is_local_name(name: &str, local_name: &str) -> bool {
    name == local_name || name.eq_ignore_ascii_case("self") || name.eq_ignore_ascii_case("local")
}

fn write_toml_atomic(path: &Path, value: &toml::Value) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create config directory {}", parent.display()))?;
    }
    let contents = toml::to_string_pretty(value)?;
    let temp = path.with_extension("toml.node.tmp");
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&temp)
        .with_context(|| format!("failed to create config file {}", temp.display()))?;
    file.write_all(contents.as_bytes())?;
    file.sync_all()?;
    fs::rename(&temp, path).with_context(|| {
        format!(
            "failed to move config file {} to {}",
            temp.display(),
            path.display()
        )
    })?;
    Ok(())
}

fn default_node_socket_path() -> Result<PathBuf> {
    let runtime_dir = std::env::var_os("XDG_RUNTIME_DIR")
        .ok_or_else(|| anyhow::anyhow!("XDG_RUNTIME_DIR is not set"))?;
    Ok(PathBuf::from(runtime_dir).join("ura/node.sock"))
}

fn local_address_for_bind(bind: SocketAddr) -> SocketAddr {
    let ip = match bind.ip() {
        IpAddr::V4(ip) if ip.is_unspecified() => IpAddr::V4(Ipv4Addr::LOCALHOST),
        IpAddr::V6(ip) if ip.is_unspecified() => IpAddr::V6(Ipv6Addr::LOCALHOST),
        ip => ip,
    };
    SocketAddr::new(ip, bind.port())
}

fn local_url_for_bind(bind: SocketAddr) -> String {
    let address = local_address_for_bind(bind);
    match address {
        SocketAddr::V4(address) => format!("http://{address}"),
        SocketAddr::V6(address) => format!("http://[{0}]:{1}", address.ip(), address.port()),
    }
}

fn prepare_socket_path(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create runtime directory {}", parent.display()))?;
        fs::set_permissions(parent, fs::Permissions::from_mode(0o700))?;
    }
    if path.exists() {
        let metadata = fs::symlink_metadata(path)?;
        if !metadata.file_type().is_socket() {
            anyhow::bail!("refusing to replace non-socket path {}", path.display());
        }
        fs::remove_file(path)?;
    }
    Ok(())
}

fn remove_socket_if_present(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("failed to remove {}", path.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_path(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time before UNIX_EPOCH")
            .as_nanos();
        std::env::temp_dir().join(format!("ura-node-{name}-{nanos}.toml"))
    }

    #[test]
    fn empty_config_resolves_to_this_device() {
        let path = unique_path("self");
        let destination = resolve_destination(Some(&path), None).expect("resolve destination");
        assert!(matches!(destination, Destination::SelfNode { .. }));
    }

    #[test]
    fn selected_peer_resolves_to_peer() {
        let path = unique_path("peer");
        fs::write(
            &path,
            r#"
selected_device = "living"

[[devices]]
name = "living"
url = "http://192.168.1.42:8765"
token = "secret"
"#,
        )
        .expect("write config");

        let destination = resolve_destination(Some(&path), None).expect("resolve destination");
        match destination {
            Destination::Peer(peer) => assert_eq!(peer.name, "living"),
            Destination::SelfNode { .. } => panic!("expected peer"),
        }
        let _ = fs::remove_file(path);
    }

    #[test]
    fn selecting_this_device_preserves_receiver_settings() {
        let path = unique_path("preserve");
        fs::write(
            &path,
            r#"
token = "receiver-token"
bind = "0.0.0.0:8765"
selected_device = "living"

[local]
name = "desktop"

[[devices]]
name = "living"
url = "http://192.168.1.42:8765"
token = "peer-token"
"#,
        )
        .expect("write config");

        let selected = select_device(Some(&path), "desktop").expect("select self");
        assert_eq!(selected, "desktop");

        let value: toml::Value = toml::from_str(&fs::read_to_string(&path).expect("read config"))
            .expect("parse config");
        let table = value.as_table().expect("config table");
        assert_eq!(
            table.get("token").and_then(toml::Value::as_str),
            Some("receiver-token")
        );
        assert_eq!(
            table.get("bind").and_then(toml::Value::as_str),
            Some("0.0.0.0:8765")
        );
        assert!(!table.contains_key("selected_device"));
        let _ = fs::remove_file(path);
    }
}
