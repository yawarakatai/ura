use std::{env, fs, path::PathBuf};

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::error::UraError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub receiver_url: String,
    pub token: String,
}

#[derive(Debug, Default, Deserialize)]
struct FileConfig {
    receiver_url: Option<String>,
    token: Option<String>,
}

impl Config {
    pub fn load_with_overrides(
        receiver_url: Option<String>,
        token: Option<String>,
    ) -> Result<Self> {
        let file_config = load_file_config()?;
        Self::from_parts(file_config, receiver_url, token)
    }

    fn from_parts(
        file_config: Option<FileConfig>,
        receiver_url: Option<String>,
        token: Option<String>,
    ) -> Result<Self> {
        let file_config = file_config.unwrap_or_default();
        let receiver_url = receiver_url
            .or(file_config.receiver_url)
            .ok_or(UraError::MissingConfigField("receiver_url"))?;
        let token = token
            .or(file_config.token)
            .ok_or(UraError::MissingConfigField("token"))?;

        Ok(Self {
            receiver_url,
            token,
        })
    }
}

fn load_file_config() -> Result<Option<FileConfig>> {
    let path = config_path()?;
    if !path.exists() {
        return Ok(None);
    }

    let contents = fs::read_to_string(&path)
        .with_context(|| format!("failed to read config file {}", path.display()))?;
    let config = toml::from_str(&contents)
        .with_context(|| format!("failed to parse config file {}", path.display()))?;
    Ok(Some(config))
}

pub fn config_path() -> Result<PathBuf> {
    if let Some(config_home) = env::var_os("XDG_CONFIG_HOME") {
        return Ok(PathBuf::from(config_home).join("ura/config.toml"));
    }

    let home = env::var_os("HOME").ok_or(UraError::MissingHome)?;
    Ok(PathBuf::from(home).join(".config/ura/config.toml"))
}

pub fn default_mpv_socket_path() -> Result<PathBuf> {
    let runtime_dir = env::var_os("XDG_RUNTIME_DIR").ok_or(UraError::MissingRuntimeDir)?;
    Ok(PathBuf::from(runtime_dir).join("ura/mpv.sock"))
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

    Config::from_parts(file_config, receiver_url, token)
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
    fn missing_config_requires_client_fields() {
        let path = unique_path("missing-config");

        let error = load_config_from_path_with_overrides(&path, None, None)
            .expect_err("missing config should fail");

        assert!(
            error.to_string().contains("missing required config field"),
            "{error:#}"
        );
    }
}
