use std::{
    env, fs,
    net::SocketAddr,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::error::UraError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Peer {
    pub name: String,
    pub url: String,
    pub token: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceConfig {
    pub local_name: Option<String>,
    pub selected_device: Option<String>,
    pub devices: Vec<Peer>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeConfig {
    pub bind: SocketAddr,
}

#[derive(Debug, Default, Deserialize)]
struct FileConfig {
    bind: Option<String>,
    local: Option<FileLocal>,
    selected_device: Option<String>,
    devices: Option<Vec<FileDevice>>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct FileLocal {
    name: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct FileDevice {
    name: String,
    url: String,
    token: String,
}

impl DeviceConfig {
    fn from_file_config(file_config: FileConfig) -> Result<Self> {
        let devices = file_config
            .devices
            .unwrap_or_default()
            .into_iter()
            .map(|device| {
                validate_device_name(&device.name)?;
                if device.token.trim().is_empty() {
                    anyhow::bail!("device `{}` token must not be empty", device.name);
                }
                Ok(Peer {
                    name: device.name,
                    url: device.url,
                    token: device.token,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        validate_unique_device_names(&devices)?;
        if let Some(selected) = &file_config.selected_device
            && !devices.iter().any(|device| &device.name == selected)
        {
            anyhow::bail!("selected device `{selected}` does not exist");
        }
        let local_name = file_config.local.and_then(|local| local.name);
        if let Some(name) = &local_name {
            validate_device_name(name)?;
        }
        Ok(Self {
            local_name,
            selected_device: file_config.selected_device,
            devices,
        })
    }

    pub fn load(config_path: Option<&Path>) -> Result<Self> {
        let file_config = load_file_config(config_path)?.unwrap_or_default();
        Self::from_file_config(file_config)
    }
}

impl NodeConfig {
    pub fn load(bind: Option<SocketAddr>) -> Result<Self> {
        let file_config = load_file_config(None)?;
        Self::from_sources(file_config, bind, env_bind())
    }

    fn from_sources(
        file_config: Option<FileConfig>,
        bind: Option<SocketAddr>,
        env_bind: Option<Result<SocketAddr>>,
    ) -> Result<Self> {
        let bind = bind
            .map(Ok)
            .or(env_bind)
            .or_else(|| bind_from_file(file_config.as_ref()))
            .transpose()?
            .unwrap_or_else(default_bind);

        Ok(Self { bind })
    }
}

pub fn generate_token() -> Result<String> {
    let mut bytes = [0_u8; 32];
    let mut random =
        fs::File::open("/dev/urandom").with_context(|| "failed to open /dev/urandom")?;
    std::io::Read::read_exact(&mut random, &mut bytes)
        .with_context(|| "failed to read random token bytes")?;
    Ok(hex_encode(&bytes))
}

pub fn normalize_peer_address(address: &str) -> Result<String> {
    let address = address.trim();
    if address.is_empty() || address.chars().any(char::is_whitespace) {
        anyhow::bail!("peer address must not be empty or contain whitespace");
    }
    let candidate = if address.contains("://") {
        address.to_string()
    } else {
        format!("http://{address}")
    };
    let rest = candidate
        .strip_prefix("http://")
        .ok_or_else(|| anyhow::anyhow!("unsupported peer address scheme"))?;
    if rest.contains("://") {
        anyhow::bail!("unsupported peer address scheme");
    }
    if rest.contains('?') || rest.contains('#') {
        anyhow::bail!("peer address must not include a query or fragment");
    }
    let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
    if !path.is_empty() {
        anyhow::bail!("peer address must not include a path");
    }
    if authority.contains('@') {
        anyhow::bail!("peer address must not include username or password");
    }
    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port)) if !port.is_empty() && port.chars().all(|ch| ch.is_ascii_digit()) => (
            host,
            port.parse::<u16>()
                .with_context(|| "invalid peer address port")?,
        ),
        Some(_) if authority.matches(':').count() > 1 => {
            anyhow::bail!("peer address must use a host name or IPv4 address")
        }
        _ => (authority, default_port()),
    };
    validate_host(host)?;
    if port == 0 {
        anyhow::bail!("peer address port must not be zero");
    }
    Ok(format!("http://{}:{port}", host.to_ascii_lowercase()))
}

pub fn default_port() -> u16 {
    8765
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

fn load_file_config(path: Option<&Path>) -> Result<Option<FileConfig>> {
    let path = config_path(path)?;
    if !path.exists() {
        return Ok(None);
    }

    let contents = fs::read_to_string(&path)
        .with_context(|| format!("failed to read config file {}", path.display()))?;
    let config = toml::from_str(&contents)
        .with_context(|| format!("failed to parse config file {}", path.display()))?;
    Ok(Some(config))
}

fn validate_unique_device_names(devices: &[Peer]) -> Result<()> {
    let mut names = std::collections::HashSet::new();
    for device in devices {
        if !names.insert(device.name.as_str()) {
            anyhow::bail!("duplicate device `{}`", device.name);
        }
    }
    Ok(())
}

fn validate_device_name(name: &str) -> Result<()> {
    if name.trim().is_empty() {
        anyhow::bail!("device name must not be empty");
    }
    Ok(())
}

fn validate_host(host: &str) -> Result<()> {
    if host.is_empty()
        || host
            .chars()
            .any(|ch| !(ch.is_ascii_alphanumeric() || ch == '.' || ch == '-'))
    {
        anyhow::bail!("invalid peer address host");
    }
    Ok(())
}

pub fn config_path(path: Option<&Path>) -> Result<PathBuf> {
    if let Some(path) = path {
        return Ok(path.to_path_buf());
    }

    if let Some(config_home) = env::var_os("XDG_CONFIG_HOME") {
        return Ok(PathBuf::from(config_home).join("ura/config.toml"));
    }

    let home = env::var_os("HOME").ok_or(UraError::MissingHome)?;
    Ok(PathBuf::from(home).join(".config/ura/config.toml"))
}

pub fn default_bind() -> SocketAddr {
    "127.0.0.1:8765"
        .parse()
        .expect("default bind address should be valid")
}

fn env_bind() -> Option<Result<SocketAddr>> {
    env::var("URA_BIND")
        .ok()
        .map(|bind| parse_bind(&bind, "URA_BIND"))
}

fn bind_from_file(file_config: Option<&FileConfig>) -> Option<Result<SocketAddr>> {
    file_config
        .and_then(|config| config.bind.as_deref())
        .map(|bind| parse_bind(bind, "config bind"))
}

fn parse_bind(bind: &str, source: &str) -> Result<SocketAddr> {
    bind.parse::<SocketAddr>()
        .with_context(|| format!("invalid {source} address `{bind}`"))
}

pub fn default_mpv_socket_path() -> Result<PathBuf> {
    let runtime_dir = env::var_os("XDG_RUNTIME_DIR").ok_or(UraError::MissingRuntimeDir)?;
    Ok(PathBuf::from(runtime_dir).join("ura/mpv.sock"))
}

pub fn default_control_socket_path() -> Result<PathBuf> {
    let runtime_dir = env::var_os("XDG_RUNTIME_DIR").ok_or(UraError::MissingRuntimeDir)?;
    Ok(PathBuf::from(runtime_dir).join("ura/control.sock"))
}

pub fn default_db_path() -> Result<PathBuf> {
    if let Some(data_home) = env::var_os("XDG_DATA_HOME") {
        return Ok(PathBuf::from(data_home).join("ura/ura.db"));
    }

    let home = env::var_os("HOME").ok_or(UraError::MissingHome)?;
    Ok(PathBuf::from(home).join(".local/share/ura/ura.db"))
}

pub fn default_device_name() -> String {
    env::var("HOSTNAME")
        .ok()
        .filter(|name| !name.trim().is_empty())
        .or_else(|| {
            fs::read_to_string("/proc/sys/kernel/hostname")
                .ok()
                .map(|name| name.trim().to_string())
                .filter(|name| !name.is_empty())
        })
        .unwrap_or_else(|| "ura".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unique_path(name: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time before UNIX_EPOCH")
            .as_nanos();
        env::temp_dir().join(format!("ura-{name}-{nanos}.toml"))
    }

    #[test]
    fn node_bind_precedence_is_cli_then_env_then_file_then_default() {
        let file = Some(FileConfig {
            bind: Some("127.0.0.1:9999".to_string()),
            ..FileConfig::default()
        });
        let env = Some(Ok("127.0.0.1:7777".parse().expect("parse env bind")));
        let cli = Some("127.0.0.1:6666".parse().expect("parse CLI bind"));

        assert_eq!(
            NodeConfig::from_sources(file, cli, env)
                .expect("resolve bind")
                .bind,
            "127.0.0.1:6666".parse().expect("parse expected bind")
        );
        assert_eq!(
            NodeConfig::from_sources(None, None, None)
                .expect("default bind")
                .bind,
            default_bind()
        );
    }

    #[test]
    fn loads_device_config_and_selection() {
        let path = unique_path("devices");
        fs::write(
            &path,
            r#"
selected_device = "kamo"

[local]
name = "desktop"

[[devices]]
name = "kamo"
url = "http://192.168.1.23:8765"
token = "kamo-token"
"#,
        )
        .expect("write config");

        let config = DeviceConfig::load(Some(&path)).expect("load config");
        assert_eq!(config.local_name.as_deref(), Some("desktop"));
        assert_eq!(config.selected_device.as_deref(), Some("kamo"));
        assert_eq!(config.devices.len(), 1);
        assert_eq!(config.devices[0].name, "kamo");

        let _ = fs::remove_file(path);
    }

    #[test]
    fn legacy_flat_peer_fields_do_not_create_a_device() {
        let path = unique_path("legacy-fields");
        fs::write(
            &path,
            r#"
receiver_url = "http://192.168.1.23:8765"
token = "legacy-token"
"#,
        )
        .expect("write config");

        let config = DeviceConfig::load(Some(&path)).expect("load config");
        assert!(config.devices.is_empty());
        assert_eq!(config.selected_device, None);

        let _ = fs::remove_file(path);
    }

    #[test]
    fn rejects_duplicate_or_missing_selected_devices() {
        let duplicate = unique_path("duplicate");
        fs::write(
            &duplicate,
            r#"
[[devices]]
name = "kamo"
url = "http://192.168.1.23:8765"
token = "one"

[[devices]]
name = "kamo"
url = "http://192.168.1.24:8765"
token = "two"
"#,
        )
        .expect("write duplicate config");
        assert!(DeviceConfig::load(Some(&duplicate)).is_err());

        let missing = unique_path("missing-selected");
        fs::write(&missing, "selected_device = \"missing\"\n").expect("write missing selection");
        assert!(DeviceConfig::load(Some(&missing)).is_err());

        let _ = fs::remove_file(duplicate);
        let _ = fs::remove_file(missing);
    }

    #[test]
    fn normalizes_peer_addresses() {
        assert_eq!(
            normalize_peer_address("192.168.1.23").expect("normalize"),
            "http://192.168.1.23:8765"
        );
        assert_eq!(
            normalize_peer_address("kamo:9999").expect("normalize"),
            "http://kamo:9999"
        );
        assert_eq!(
            normalize_peer_address("http://kamo/").expect("normalize"),
            "http://kamo:8765"
        );
    }

    #[test]
    fn rejects_invalid_peer_addresses() {
        for address in [
            "https://kamo",
            "http://kamo/path",
            "http://kamo?query=1",
            "http://kamo#fragment",
            "http://user@kamo",
        ] {
            normalize_peer_address(address).expect_err("address should fail");
        }
    }

    #[test]
    fn generated_tokens_are_suitable_for_internal_node_authentication() {
        let token = generate_token().expect("generate token");

        assert!(token.len() >= 64);
        crate::peer_api::validate_peer_token(&token).expect("generated token is valid");
    }
}
