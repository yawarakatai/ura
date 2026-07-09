use std::{
    env, fs,
    fs::OpenOptions,
    io::Write,
    net::SocketAddr,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::error::UraError;
use crate::receiver::validate_receiver_token;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub receiver_url: String,
    pub token: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceiverConfig {
    pub bind: SocketAddr,
    pub token: String,
}

#[derive(Debug, Default, Deserialize)]
struct FileConfig {
    receiver_url: Option<String>,
    token: Option<String>,
    bind: Option<String>,
}

impl Config {
    pub fn load_with_overrides(
        config_path: Option<PathBuf>,
        receiver_url: Option<String>,
        token: Option<String>,
    ) -> Result<Self> {
        let file_config = load_file_config(config_path.as_deref())?;
        Self::from_sources(
            file_config,
            receiver_url,
            token,
            env_receiver_url(),
            env_token(),
        )
    }

    fn from_sources(
        file_config: Option<FileConfig>,
        receiver_url: Option<String>,
        token: Option<String>,
        env_receiver_url: Option<String>,
        env_token: Option<String>,
    ) -> Result<Self> {
        let file_config = file_config.unwrap_or_default();
        let receiver_url = receiver_url
            .or(env_receiver_url)
            .or(file_config.receiver_url)
            .ok_or(UraError::MissingConfigField("receiver_url"))?;
        let token = token
            .or(env_token)
            .or(file_config.token)
            .ok_or(UraError::MissingConfigField("token"))?;

        Ok(Self {
            receiver_url,
            token,
        })
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

impl ReceiverConfig {
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
        let token = token
            .or_else(|| file_config.and_then(|config| config.token))
            .ok_or(UraError::MissingConfigField("token"))?;

        Ok(Self { bind, token })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigInit {
    pub config_path: Option<PathBuf>,
    pub receiver_url: String,
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
    validate_receiver_token(&token)?;

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create config directory {}", parent.display()))?;
    }

    let contents = format!(
        "token = \"{}\"\nreceiver_url = \"{}\"\nbind = \"{}\"\n",
        toml_escape_string(&token),
        toml_escape_string(&init.receiver_url),
        init.bind
    );

    let mut options = OpenOptions::new();
    options.write(true);
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

fn env_receiver_url() -> Option<String> {
    env::var("URA_RECEIVER_URL").ok()
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

pub fn default_db_path() -> Result<PathBuf> {
    if let Some(data_home) = env::var_os("XDG_DATA_HOME") {
        return Ok(PathBuf::from(data_home).join("ura/ura.db"));
    }

    let home = env::var_os("HOME").ok_or(UraError::MissingHome)?;
    Ok(PathBuf::from(home).join(".local/share/ura/ura.db"))
}

#[cfg(test)]
pub fn load_config_from_path_with_overrides(
    path: &std::path::Path,
    receiver_url: Option<String>,
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

    Config::from_sources(file_config, receiver_url, token, None, None)
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
receiver_url = "http://127.0.0.1:8765"
token = "secret"
"#,
        )
        .expect("write test config");

        let config =
            load_config_from_path_with_overrides(&path, None, None).expect("load test config");

        assert_eq!(
            config,
            Config {
                receiver_url: "http://127.0.0.1:8765".to_string(),
                token: "secret".to_string(),
            }
        );

        let _ = fs::remove_file(path);
    }

    #[test]
    fn cli_overrides_config_file() {
        let path = unique_path("cli-overrides");
        fs::write(
            &path,
            r#"
receiver_url = "http://127.0.0.1:8765"
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

        assert_eq!(config.receiver_url, "http://receiver.example");
        assert_eq!(config.token, "override");

        let _ = fs::remove_file(path);
    }

    #[test]
    fn env_overrides_client_config_file() {
        let file_config = Some(FileConfig {
            receiver_url: Some("http://config.example".to_string()),
            token: Some("config-token".to_string()),
            bind: None,
        });

        let config = Config::from_sources(
            file_config,
            None,
            None,
            Some("http://env.example".to_string()),
            Some("env-token".to_string()),
        )
        .expect("load config");

        assert_eq!(config.receiver_url, "http://env.example");
        assert_eq!(config.token, "env-token");
    }

    #[test]
    fn cli_overrides_env_and_client_config_file() {
        let file_config = Some(FileConfig {
            receiver_url: Some("http://config.example".to_string()),
            token: Some("config-token".to_string()),
            bind: None,
        });

        let config = Config::from_sources(
            file_config,
            Some("http://cli.example".to_string()),
            Some("cli-token".to_string()),
            Some("http://env.example".to_string()),
            Some("env-token".to_string()),
        )
        .expect("load config");

        assert_eq!(config.receiver_url, "http://cli.example");
        assert_eq!(config.token, "cli-token");
    }

    #[test]
    fn receiver_reads_token_and_bind_from_config_file() {
        let config = ReceiverConfig::from_sources(
            Some(FileConfig {
                receiver_url: None,
                token: Some("0123456789abcdef0123456789abcdef".to_string()),
                bind: Some("127.0.0.1:9999".to_string()),
            }),
            None,
            None,
            None,
        )
        .expect("load receiver config");

        assert_eq!(config.bind, "127.0.0.1:9999".parse().expect("parse bind"));
        assert_eq!(config.token, "0123456789abcdef0123456789abcdef");
    }

    #[test]
    fn receiver_defaults_bind_when_config_omits_it() {
        let config = ReceiverConfig::from_sources(
            Some(FileConfig {
                receiver_url: None,
                token: Some("0123456789abcdef0123456789abcdef".to_string()),
                bind: None,
            }),
            None,
            None,
            None,
        )
        .expect("load receiver config");

        assert_eq!(config.bind, default_bind());
    }

    #[test]
    fn env_overrides_receiver_config_file() {
        let config = ReceiverConfig::from_sources(
            Some(FileConfig {
                receiver_url: None,
                token: Some("config-token".to_string()),
                bind: Some("127.0.0.1:9999".to_string()),
            }),
            None,
            Some("env-token".to_string()),
            Some(Ok("127.0.0.1:7777".parse().expect("parse bind"))),
        )
        .expect("load receiver config");

        assert_eq!(config.bind, "127.0.0.1:7777".parse().expect("parse bind"));
        assert_eq!(config.token, "env-token");
    }

    #[test]
    fn cli_overrides_env_and_receiver_config_file() {
        let config = ReceiverConfig::from_sources(
            Some(FileConfig {
                receiver_url: None,
                token: Some("config-token".to_string()),
                bind: Some("127.0.0.1:9999".to_string()),
            }),
            Some("127.0.0.1:6666".parse().expect("parse bind")),
            Some("cli-token".to_string()),
            Some(Ok("127.0.0.1:7777".parse().expect("parse bind"))),
        )
        .expect("load receiver config");

        assert_eq!(config.bind, "127.0.0.1:6666".parse().expect("parse bind"));
        assert_eq!(config.token, "cli-token");
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
    fn receive_can_load_token_without_receiver_url() {
        let file_config = FileConfig {
            receiver_url: None,
            token: Some("secret".to_string()),
            bind: None,
        };

        let token = Config::token_from_parts(file_config, None).expect("load receiver token");

        assert_eq!(token, "secret");
    }

    #[test]
    fn init_config_creates_config_with_generated_token() {
        let path = unique_path("init-config");

        let created = init_config_at_path(
            &path,
            ConfigInit {
                config_path: None,
                receiver_url: "http://127.0.0.1:8765".to_string(),
                bind: default_bind(),
                token: None,
                force: false,
            },
        )
        .expect("init config");

        assert_eq!(created, path);
        let config =
            load_config_from_path_with_overrides(&created, None, None).expect("load new config");
        assert_eq!(config.receiver_url, "http://127.0.0.1:8765");
        assert!(config.token.len() >= 32);
        crate::receiver::validate_receiver_token(&config.token).expect("generated token is valid");
        assert!(
            fs::read_to_string(&created)
                .expect("read new config")
                .contains(r#"bind = "127.0.0.1:8765""#)
        );

        let _ = fs::remove_file(created);
    }

    #[test]
    fn token_generate_produces_valid_receiver_token() {
        let token = generate_token().expect("generate token");

        assert!(token.len() >= 32);
        crate::receiver::validate_receiver_token(&token).expect("generated token is valid");
    }

    #[test]
    fn init_config_refuses_to_overwrite_without_force() {
        let path = unique_path("init-existing");
        fs::write(
            &path,
            r#"
receiver_url = "http://old.example"
token = "old-token"
"#,
        )
        .expect("write existing config");

        let error = init_config_at_path(
            &path,
            ConfigInit {
                config_path: None,
                receiver_url: "http://127.0.0.1:8765".to_string(),
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
receiver_url = "http://old.example"
token = "0123456789abcdef0123456789abcdef"
"#,
        )
        .expect("write existing config");

        init_config_at_path(
            &path,
            ConfigInit {
                config_path: None,
                receiver_url: "http://127.0.0.1:8765".to_string(),
                bind: default_bind(),
                token: Some("abcdef0123456789abcdef0123456789".to_string()),
                force: true,
            },
        )
        .expect("force init config");

        let config = load_config_from_path_with_overrides(&path, None, None)
            .expect("load overwritten config");
        assert_eq!(config.receiver_url, "http://127.0.0.1:8765");
        assert_eq!(config.token, "abcdef0123456789abcdef0123456789");

        let _ = fs::remove_file(path);
    }
}
