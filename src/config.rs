use std::{
    env, fs,
    fs::OpenOptions,
    io::Write,
    net::SocketAddr,
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::error::UraError;
use crate::peer_api::validate_peer_token;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub peer_url: String,
    pub token: String,
}

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
    pub legacy_peer: Option<Peer>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeConfig {
    pub bind: SocketAddr,
    pub token: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct FileConfig {
    #[serde(alias = "receiver_url")]
    peer_url: Option<String>,
    token: Option<String>,
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

impl Config {
    pub fn load_with_overrides(
        config_path: Option<PathBuf>,
        peer_url: Option<String>,
        token: Option<String>,
    ) -> Result<Self> {
        let file_config = load_file_config(config_path.as_deref())?;
        Self::from_sources(file_config, peer_url, token, env_peer_url(), env_token())
    }

    pub fn load_for_device(config_path: Option<PathBuf>, to: Option<String>) -> Result<Self> {
        let file_config = load_file_config(config_path.as_deref())?.unwrap_or_default();
        let device = DeviceConfig::from_file_config(file_config)?.resolve(to.as_deref())?;
        Ok(Self {
            peer_url: device.url,
            token: device.token,
        })
    }

    fn from_sources(
        file_config: Option<FileConfig>,
        peer_url: Option<String>,
        token: Option<String>,
        env_peer_url: Option<String>,
        env_token: Option<String>,
    ) -> Result<Self> {
        let file_config = file_config.unwrap_or_default();
        let peer_url = peer_url
            .or(env_peer_url)
            .or(file_config.peer_url)
            .ok_or(UraError::MissingConfigField("peer_url"))?;
        let token = token
            .or(env_token)
            .or(file_config.token)
            .ok_or(UraError::MissingConfigField("token"))?;

        Ok(Self { peer_url, token })
    }

    pub fn load_token_with_override(token: Option<String>) -> Result<String> {
        let file_config = load_file_config(None)?.unwrap_or_default();
        let token = token.or_else(env_token);
        Self::token_from_parts(file_config, token)
    }

    fn token_from_parts(file_config: FileConfig, token: Option<String>) -> Result<String> {
        token
            .or(file_config.token)
            .ok_or_else(|| UraError::MissingConfigField("token").into())
    }

    pub fn init(init: ConfigInit) -> Result<PathBuf> {
        let path = config_path(init.config_path.as_deref())?;
        init_config_at_path(&path, init)
    }
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
        let legacy_peer = match (file_config.peer_url, file_config.token) {
            (Some(url), Some(token)) if devices.is_empty() => Some(Peer {
                name: "legacy".to_string(),
                url,
                token,
            }),
            _ => None,
        };
        Ok(Self {
            local_name,
            selected_device: file_config.selected_device,
            devices,
            legacy_peer,
        })
    }

    pub fn load(config_path: Option<&Path>) -> Result<Self> {
        let file_config = load_file_config(config_path)?.unwrap_or_default();
        Self::from_file_config(file_config)
    }

    pub fn resolve(&self, to: Option<&str>) -> Result<Peer> {
        if let Some(name) = to {
            return self
                .devices
                .iter()
                .find(|device| device.name == name)
                .cloned()
                .or_else(|| {
                    self.legacy_peer
                        .as_ref()
                        .filter(|device| device.name == name)
                        .cloned()
                })
                .ok_or_else(|| anyhow::anyhow!("unknown device `{name}`"));
        }

        if let Some(selected) = &self.selected_device {
            return self
                .devices
                .iter()
                .find(|device| &device.name == selected)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("selected device `{selected}` does not exist"));
        }

        if let Some(legacy) = &self.legacy_peer {
            return Ok(legacy.clone());
        }

        anyhow::bail!(
            "no device selected\n\nUse:\n  ura device select <name>\nor:\n  ura play --to <name> <url>"
        );
    }

    pub fn add(config_path: Option<&Path>, name: &str, address: &str, token: &str) -> Result<()> {
        validate_device_name(name)?;
        if token.trim().is_empty() {
            anyhow::bail!("device token must not be empty");
        }
        let path = config_path_path(config_path)?;
        let mut config = Self::load(config_path)?;
        if config.devices.iter().any(|device| device.name == name) {
            anyhow::bail!("device `{name}` already exists");
        }
        config.devices.push(Peer {
            name: name.to_string(),
            url: normalize_peer_address(address)?,
            token: token.to_string(),
        });
        write_device_config(&path, &config)
    }

    pub fn add_paired(
        config_path: Option<&Path>,
        peer_alias: &str,
        address: &str,
        token: &str,
        local_name: &str,
        select: bool,
    ) -> Result<()> {
        validate_device_name(peer_alias)?;
        validate_device_name(local_name)?;
        if token.trim().is_empty() {
            anyhow::bail!("device token must not be empty");
        }
        let path = config_path_path(config_path)?;
        let mut config = Self::load(config_path)?;
        if config
            .devices
            .iter()
            .any(|device| device.name == peer_alias)
        {
            anyhow::bail!("device `{peer_alias}` already exists");
        }
        config.local_name = Some(local_name.to_string());
        config.devices.push(Peer {
            name: peer_alias.to_string(),
            url: normalize_peer_address(address)?,
            token: token.to_string(),
        });
        if select {
            config.selected_device = Some(peer_alias.to_string());
        }
        write_device_config(&path, &config)
    }

    pub fn select(config_path: Option<&Path>, name: &str) -> Result<()> {
        let path = config_path_path(config_path)?;
        let mut config = Self::load(config_path)?;
        if !config.devices.iter().any(|device| device.name == name) {
            anyhow::bail!("unknown device `{name}`");
        }
        config.selected_device = Some(name.to_string());
        write_device_config(&path, &config)
    }

    pub fn remove(config_path: Option<&Path>, name: &str) -> Result<()> {
        let path = config_path_path(config_path)?;
        let mut config = Self::load(config_path)?;
        let before = config.devices.len();
        config.devices.retain(|device| device.name != name);
        if config.devices.len() == before {
            anyhow::bail!("unknown device `{name}`");
        }
        if config.selected_device.as_deref() == Some(name) {
            config.selected_device = None;
        }
        write_device_config(&path, &config)
    }
}

impl NodeConfig {
    pub fn load_with_overrides(
        config_path: Option<PathBuf>,
        bind: Option<SocketAddr>,
        token: Option<String>,
    ) -> Result<Self> {
        let file_config = load_file_config(config_path.as_deref())?;
        Self::from_sources(file_config, bind, token.or_else(env_token), env_bind())
    }

    fn from_sources(
        file_config: Option<FileConfig>,
        bind: Option<SocketAddr>,
        token: Option<String>,
        env_bind: Option<Result<SocketAddr>>,
    ) -> Result<Self> {
        let bind = bind
            .map(Ok)
            .or(env_bind)
            .or_else(|| bind_from_file(file_config.as_ref()))
            .transpose()?
            .unwrap_or_else(default_bind);
        let token = token.or_else(|| file_config.and_then(|config| config.token));

        Ok(Self { bind, token })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigInit {
    pub config_path: Option<PathBuf>,
    pub peer_url: String,
    pub bind: SocketAddr,
    pub token: Option<String>,
    pub force: bool,
}

pub fn generate_token() -> Result<String> {
    let mut bytes = [0_u8; 32];
    let mut random =
        fs::File::open("/dev/urandom").with_context(|| "failed to open /dev/urandom")?;
    std::io::Read::read_exact(&mut random, &mut bytes)
        .with_context(|| "failed to read random token bytes")?;
    Ok(hex_encode(&bytes))
}

fn init_config_at_path(path: &Path, init: ConfigInit) -> Result<PathBuf> {
    let token = match init.token {
        Some(token) => token,
        None => generate_token()?,
    };
    validate_peer_token(&token)?;

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create config directory {}", parent.display()))?;
    }

    let contents = format!(
        "token = \"{}\"\npeer_url = \"{}\"\nbind = \"{}\"\n",
        toml_escape_string(&token),
        toml_escape_string(&init.peer_url),
        init.bind
    );

    let mut options = OpenOptions::new();
    options.write(true);
    options.mode(0o600);
    if init.force {
        options.create(true).truncate(true);
    } else {
        options.create_new(true);
    }

    let mut file = options
        .open(path)
        .with_context(|| format!("failed to create config file {}", path.display()))?;
    file.write_all(contents.as_bytes())
        .with_context(|| format!("failed to write config file {}", path.display()))?;

    Ok(path.to_path_buf())
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
                .with_context(|| "invalid receiver address port")?,
        ),
        Some(_) if authority.matches(':').count() > 1 => {
            anyhow::bail!("receiver address must use a host name or IPv4 address")
        }
        _ => (authority, default_port()),
    };
    validate_host(host)?;
    if port == 0 {
        anyhow::bail!("receiver address port must not be zero");
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

fn toml_escape_string(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            ch => escaped.push(ch),
        }
    }
    escaped
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

fn write_device_config(path: &Path, config: &DeviceConfig) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create config directory {}", parent.display()))?;
    }
    validate_unique_device_names(&config.devices)?;
    if let Some(selected) = &config.selected_device
        && !config.devices.iter().any(|device| &device.name == selected)
    {
        anyhow::bail!("selected device `{selected}` does not exist");
    }

    let mut contents = String::new();
    if let Some(selected) = &config.selected_device {
        contents.push_str(&format!(
            "selected_device = \"{}\"\n\n",
            toml_escape_string(selected)
        ));
    }
    if let Some(local_name) = &config.local_name {
        validate_device_name(local_name)?;
        contents.push_str("[local]\n");
        contents.push_str(&format!(
            "name = \"{}\"\n\n",
            toml_escape_string(local_name)
        ));
    }
    for device in &config.devices {
        validate_device_name(&device.name)?;
        if device.token.trim().is_empty() {
            anyhow::bail!("device `{}` token must not be empty", device.name);
        }
        contents.push_str("[[devices]]\n");
        contents.push_str(&format!(
            "name = \"{}\"\n",
            toml_escape_string(&device.name)
        ));
        contents.push_str(&format!("url = \"{}\"\n", toml_escape_string(&device.url)));
        contents.push_str(&format!(
            "token = \"{}\"\n\n",
            toml_escape_string(&device.token)
        ));
    }

    let temp_path = path.with_extension("toml.tmp");
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&temp_path)
        .with_context(|| format!("failed to create config file {}", temp_path.display()))?;
    file.write_all(contents.as_bytes())
        .with_context(|| format!("failed to write config file {}", temp_path.display()))?;
    file.sync_all()
        .with_context(|| format!("failed to sync config file {}", temp_path.display()))?;
    fs::rename(&temp_path, path).with_context(|| {
        format!(
            "failed to move config file {} to {}",
            temp_path.display(),
            path.display()
        )
    })?;
    Ok(())
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

fn config_path_path(path: Option<&Path>) -> Result<PathBuf> {
    config_path(path)
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

fn env_peer_url() -> Option<String> {
    env::var("URA_PEER_URL")
        .ok()
        .or_else(|| env::var("URA_RECEIVER_URL").ok())
}

fn env_token() -> Option<String> {
    env::var("URA_TOKEN").ok()
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
pub fn load_config_from_path_with_overrides(
    path: &std::path::Path,
    peer_url: Option<String>,
    token: Option<String>,
) -> Result<Config> {
    let file_config = if path.exists() {
        let contents = fs::read_to_string(path)
            .with_context(|| format!("failed to read config file {}", path.display()))?;
        Some(
            toml::from_str(&contents)
                .with_context(|| format!("failed to parse config file {}", path.display()))?,
        )
    } else {
        None
    };

    Config::from_sources(file_config, peer_url, token, None, None)
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
    fn loads_config_file() {
        let path = unique_path("loads-config-file");
        fs::write(
            &path,
            r#"
peer_url = "http://127.0.0.1:8765"
token = "secret"
"#,
        )
        .expect("write test config");

        let config =
            load_config_from_path_with_overrides(&path, None, None).expect("load test config");

        assert_eq!(
            config,
            Config {
                peer_url: "http://127.0.0.1:8765".to_string(),
                token: "secret".to_string(),
            }
        );

        let _ = fs::remove_file(path);
    }

    #[test]
    fn loads_legacy_receiver_url_key() {
        let path = unique_path("legacy-receiver-url");
        fs::write(
            &path,
            r#"
receiver_url = "http://127.0.0.1:8765"
token = "secret"
"#,
        )
        .expect("write legacy config");

        let config =
            load_config_from_path_with_overrides(&path, None, None).expect("load legacy config");
        assert_eq!(config.peer_url, "http://127.0.0.1:8765");

        let _ = fs::remove_file(path);
    }

    #[test]
    fn cli_overrides_config_file() {
        let path = unique_path("cli-overrides");
        fs::write(
            &path,
            r#"
peer_url = "http://127.0.0.1:8765"
token = "secret"
"#,
        )
        .expect("write test config");

        let config = load_config_from_path_with_overrides(
            &path,
            Some("http://receiver.example".to_string()),
            Some("override".to_string()),
        )
        .expect("load test config");

        assert_eq!(config.peer_url, "http://receiver.example");
        assert_eq!(config.token, "override");

        let _ = fs::remove_file(path);
    }

    #[test]
    fn env_overrides_client_config_file() {
        let file_config = Some(FileConfig {
            peer_url: Some("http://config.example".to_string()),
            token: Some("config-token".to_string()),
            bind: None,
            ..FileConfig::default()
        });

        let config = Config::from_sources(
            file_config,
            None,
            None,
            Some("http://env.example".to_string()),
            Some("env-token".to_string()),
        )
        .expect("load config");

        assert_eq!(config.peer_url, "http://env.example");
        assert_eq!(config.token, "env-token");
    }

    #[test]
    fn cli_overrides_env_and_client_config_file() {
        let file_config = Some(FileConfig {
            peer_url: Some("http://config.example".to_string()),
            token: Some("config-token".to_string()),
            bind: None,
            ..FileConfig::default()
        });

        let config = Config::from_sources(
            file_config,
            Some("http://cli.example".to_string()),
            Some("cli-token".to_string()),
            Some("http://env.example".to_string()),
            Some("env-token".to_string()),
        )
        .expect("load config");

        assert_eq!(config.peer_url, "http://cli.example");
        assert_eq!(config.token, "cli-token");
    }

    #[test]
    fn node_reads_token_and_bind_from_config_file() {
        let config = NodeConfig::from_sources(
            Some(FileConfig {
                peer_url: None,
                token: Some("0123456789abcdef0123456789abcdef".to_string()),
                bind: Some("127.0.0.1:9999".to_string()),
                ..FileConfig::default()
            }),
            None,
            None,
            None,
        )
        .expect("load node config");

        assert_eq!(config.bind, "127.0.0.1:9999".parse().expect("parse bind"));
        assert_eq!(
            config.token.as_deref(),
            Some("0123456789abcdef0123456789abcdef")
        );
    }

    #[test]
    fn node_defaults_bind_when_config_omits_it() {
        let config = NodeConfig::from_sources(
            Some(FileConfig {
                peer_url: None,
                token: Some("0123456789abcdef0123456789abcdef".to_string()),
                bind: None,
                ..FileConfig::default()
            }),
            None,
            None,
            None,
        )
        .expect("load node config");

        assert_eq!(config.bind, default_bind());
    }

    #[test]
    fn node_can_omit_legacy_token() {
        let config = NodeConfig::from_sources(
            Some(FileConfig {
                peer_url: None,
                token: None,
                bind: None,
                ..FileConfig::default()
            }),
            None,
            None,
            None,
        )
        .expect("load node config");

        assert_eq!(config.token, None);
    }

    #[test]
    fn env_overrides_node_config_file() {
        let config = NodeConfig::from_sources(
            Some(FileConfig {
                peer_url: None,
                token: Some("config-token".to_string()),
                bind: Some("127.0.0.1:9999".to_string()),
                ..FileConfig::default()
            }),
            None,
            Some("env-token".to_string()),
            Some(Ok("127.0.0.1:7777".parse().expect("parse bind"))),
        )
        .expect("load node config");

        assert_eq!(config.bind, "127.0.0.1:7777".parse().expect("parse bind"));
        assert_eq!(config.token.as_deref(), Some("env-token"));
    }

    #[test]
    fn cli_overrides_env_and_node_config_file() {
        let config = NodeConfig::from_sources(
            Some(FileConfig {
                peer_url: None,
                token: Some("config-token".to_string()),
                bind: Some("127.0.0.1:9999".to_string()),
                ..FileConfig::default()
            }),
            Some("127.0.0.1:6666".parse().expect("parse bind")),
            Some("cli-token".to_string()),
            Some(Ok("127.0.0.1:7777".parse().expect("parse bind"))),
        )
        .expect("load node config");

        assert_eq!(config.bind, "127.0.0.1:6666".parse().expect("parse bind"));
        assert_eq!(config.token.as_deref(), Some("cli-token"));
    }

    #[test]
    fn missing_config_requires_client_fields() {
        let path = unique_path("missing-config");

        let error = load_config_from_path_with_overrides(&path, None, None)
            .expect_err("missing config should fail");

        assert!(
            error.to_string().contains("missing required config field"),
            "{error:#}"
        );
    }

    #[test]
    fn node_can_load_legacy_token_without_peer_url() {
        let file_config = FileConfig {
            peer_url: None,
            token: Some("secret".to_string()),
            bind: None,
            ..FileConfig::default()
        };

        let token = Config::token_from_parts(file_config, None).expect("load peer API token");

        assert_eq!(token, "secret");
    }

    #[test]
    fn init_config_creates_config_with_generated_token() {
        let path = unique_path("init-config");

        let created = init_config_at_path(
            &path,
            ConfigInit {
                config_path: None,
                peer_url: "http://127.0.0.1:8765".to_string(),
                bind: default_bind(),
                token: None,
                force: false,
            },
        )
        .expect("init config");

        assert_eq!(created, path);
        let config =
            load_config_from_path_with_overrides(&created, None, None).expect("load new config");
        assert_eq!(config.peer_url, "http://127.0.0.1:8765");
        assert!(config.token.len() >= 32);
        crate::peer_api::validate_peer_token(&config.token).expect("generated token is valid");
        assert!(
            fs::read_to_string(&created)
                .expect("read new config")
                .contains(r#"bind = "127.0.0.1:8765""#)
        );

        let _ = fs::remove_file(created);
    }

    #[test]
    fn token_generate_produces_valid_peer_api_token() {
        let token = generate_token().expect("generate token");

        assert!(token.len() >= 32);
        crate::peer_api::validate_peer_token(&token).expect("generated token is valid");
    }

    #[test]
    fn init_config_refuses_to_overwrite_without_force() {
        let path = unique_path("init-existing");
        fs::write(
            &path,
            r#"
peer_url = "http://old.example"
token = "old-token"
"#,
        )
        .expect("write existing config");

        let error = init_config_at_path(
            &path,
            ConfigInit {
                config_path: None,
                peer_url: "http://127.0.0.1:8765".to_string(),
                bind: default_bind(),
                token: Some("0123456789abcdef0123456789abcdef".to_string()),
                force: false,
            },
        )
        .expect_err("existing config should not be overwritten");

        assert!(
            error.to_string().contains("failed to create config file"),
            "{error:#}"
        );
        assert!(
            fs::read_to_string(&path)
                .expect("read existing config")
                .contains("http://old.example")
        );

        let _ = fs::remove_file(path);
    }

    #[test]
    fn init_config_force_overwrites_existing_config() {
        let path = unique_path("init-force");
        fs::write(
            &path,
            r#"
peer_url = "http://old.example"
token = "0123456789abcdef0123456789abcdef"
"#,
        )
        .expect("write existing config");

        init_config_at_path(
            &path,
            ConfigInit {
                config_path: None,
                peer_url: "http://127.0.0.1:8765".to_string(),
                bind: default_bind(),
                token: Some("abcdef0123456789abcdef0123456789".to_string()),
                force: true,
            },
        )
        .expect("force init config");

        let config = load_config_from_path_with_overrides(&path, None, None)
            .expect("load overwritten config");
        assert_eq!(config.peer_url, "http://127.0.0.1:8765");
        assert_eq!(config.token, "abcdef0123456789abcdef0123456789");

        let _ = fs::remove_file(path);
    }

    #[test]
    fn normalizes_peer_addresses() {
        assert_eq!(
            normalize_peer_address("192.168.1.23").expect("normalize"),
            "http://192.168.1.23:8765"
        );
        assert_eq!(
            normalize_peer_address("http://192.168.1.23").expect("normalize"),
            "http://192.168.1.23:8765"
        );
        assert_eq!(
            normalize_peer_address("kamo:9999").expect("normalize"),
            "http://kamo:9999"
        );
        assert_eq!(
            normalize_peer_address("kamo.local").expect("normalize"),
            "http://kamo.local:8765"
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
    fn parses_multiple_device_config_and_resolves_selection() {
        let path = unique_path("multi-device");
        fs::write(
            &path,
            r#"
selected_device = "kamo"

[[devices]]
name = "kamo"
url = "http://192.168.1.23:8765"
token = "kamo-token"

[[devices]]
name = "dane"
url = "http://192.168.1.42:8765"
token = "dane-token"
"#,
        )
        .expect("write config");

        let config = DeviceConfig::load(Some(&path)).expect("load devices");
        assert_eq!(config.devices.len(), 2);
        assert_eq!(config.resolve(None).expect("selected").name, "kamo");
        assert_eq!(
            config.resolve(Some("dane")).expect("--to").token,
            "dane-token"
        );
        assert!(config.resolve(Some("unknown")).is_err());

        let _ = fs::remove_file(path);
    }

    #[test]
    fn validates_unique_device_names() {
        let path = unique_path("duplicate-device");
        fs::write(
            &path,
            r#"
[[devices]]
name = "kamo"
url = "http://192.168.1.23:8765"
token = "kamo-token"

[[devices]]
name = "kamo"
url = "http://192.168.1.42:8765"
token = "dane-token"
"#,
        )
        .expect("write config");

        DeviceConfig::load(Some(&path)).expect_err("duplicate should fail");

        let _ = fs::remove_file(path);
    }

    #[test]
    fn no_selected_device_error_is_actionable() {
        let config = DeviceConfig {
            local_name: None,
            selected_device: None,
            devices: vec![Peer {
                name: "kamo".to_string(),
                url: "http://kamo:8765".to_string(),
                token: "token".to_string(),
            }],
            legacy_peer: None,
        };

        let error = config.resolve(None).expect_err("missing selection");

        assert!(error.to_string().contains("ura device select <name>"));
        assert!(error.to_string().contains("ura play --to <name> <url>"));
    }

    #[test]
    fn device_add_select_and_remove_update_config() {
        let path = unique_path("device-ops");

        DeviceConfig::add(Some(&path), "kamo", "192.168.1.23", "secret-token").expect("add device");
        DeviceConfig::add(Some(&path), "dane", "192.168.1.42:9999", "dane-token")
            .expect("add device");
        DeviceConfig::select(Some(&path), "kamo").expect("select");

        let config = DeviceConfig::load(Some(&path)).expect("load devices");
        assert_eq!(config.selected_device.as_deref(), Some("kamo"));
        assert_eq!(config.devices[0].url, "http://192.168.1.23:8765");
        assert_eq!(config.devices[1].url, "http://192.168.1.42:9999");

        DeviceConfig::remove(Some(&path), "kamo").expect("remove selected");
        let config = DeviceConfig::load(Some(&path)).expect("reload devices");
        assert_eq!(config.selected_device, None);
        assert_eq!(config.devices.len(), 1);
        assert_eq!(config.devices[0].name, "dane");

        let contents = fs::read_to_string(&path).expect("read config");
        assert!(contents.contains("dane-token"));

        let _ = fs::remove_file(path);
    }

    #[test]
    fn paired_device_add_stores_local_name_and_optional_selection() {
        let path = unique_path("paired-device");

        DeviceConfig::add_paired(
            Some(&path),
            "kamo",
            "192.168.1.23",
            "pair-token",
            "desuwa",
            true,
        )
        .expect("add paired device");

        let config = DeviceConfig::load(Some(&path)).expect("load devices");
        assert_eq!(config.local_name.as_deref(), Some("desuwa"));
        assert_eq!(config.selected_device.as_deref(), Some("kamo"));
        assert_eq!(config.devices[0].url, "http://192.168.1.23:8765");

        let contents = fs::read_to_string(&path).expect("read config");
        assert!(contents.contains("[local]"));
        assert!(contents.contains("name = \"desuwa\""));

        let _ = fs::remove_file(path);
    }

    #[test]
    fn legacy_flat_config_falls_back_without_rewrite() {
        let path = unique_path("legacy-config");
        let contents = r#"
peer_url = "http://127.0.0.1:8765"
token = "legacy-token"
"#;
        fs::write(&path, contents).expect("write legacy config");

        let config = Config::load_for_device(Some(path.clone()), None).expect("load legacy");

        assert_eq!(config.peer_url, "http://127.0.0.1:8765");
        assert_eq!(config.token, "legacy-token");
        assert_eq!(
            fs::read_to_string(&path).expect("read legacy config"),
            contents
        );

        let _ = fs::remove_file(path);
    }
}
