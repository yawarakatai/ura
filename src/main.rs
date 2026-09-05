use std::{
    io::{self, IsTerminal, Read, Write},
    net::Shutdown,
    os::unix::net::UnixStream,
    time::Duration,
};

use anyhow::{Context, Result};
use clap::Parser;
use serde::{Deserialize, Serialize};
use ura::{
    cli::{Cli, Command, ConfigCommand, DeviceCommand, LoopCommand, TokenCommand},
    client::{PeerClient, PeerPairingClient},
    config::{
        Config, ConfigInit, DeviceConfig, NodeConfig, default_control_socket_path, default_db_path,
        default_device_name, generate_token, normalize_peer_address,
    },
    db::{Database, HistoryEntry, authorize_client},
    mpv::{LoopStatus, MpvStatus},
    node::{
        Destination, DeviceKind, DeviceSet, NodeClient, add_paired_peer, add_peer, device_set,
        remove_peer, resolve_destination, run_node, select_device,
    },
    pairing::{PairingCompletion, PairingStatus, valid_pairing_code},
    selector,
};

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Daemon { bind } => {
            init_daemon_logging();
            let config_path = cli.config.clone();
            let config = NodeConfig::load_with_overrides(cli.config, bind, cli.token)?;
            run_node(config.bind, config.token, config_path).await
        }
        Command::Play { to, url } => {
            let client = playback_client_for(cli.config, cli.peer_url, cli.token, to)?;
            client.play(&url)?;
            println!("play: sent {url}");
            Ok(())
        }
        Command::Queue { to, url } => {
            let client = playback_client_for(cli.config, cli.peer_url, cli.token, to)?;
            client.queue(&url)?;
            println!("queue: sent {url}");
            Ok(())
        }
        Command::Pause { to } => {
            playback_client_for(cli.config, cli.peer_url, cli.token, to)?.control("pause")?;
            println!("pause: sent");
            Ok(())
        }
        Command::Resume { to } => {
            playback_client_for(cli.config, cli.peer_url, cli.token, to)?.control("resume")?;
            println!("resume: sent");
            Ok(())
        }
        Command::Toggle { to } => {
            playback_client_for(cli.config, cli.peer_url, cli.token, to)?.control("toggle")?;
            println!("toggle: sent");
            Ok(())
        }
        Command::Stop { to } => {
            playback_client_for(cli.config, cli.peer_url, cli.token, to)?.control("stop")?;
            println!("stop: sent");
            Ok(())
        }
        Command::Loop { to, command } => {
            let to = command_to_device(&to, &command);
            let client = playback_client_for(cli.config, cli.peer_url, cli.token, to)?;
            match command {
                None => {
                    if client.loop_status()? == LoopStatus::One {
                        client.control("loop-off")?;
                        println!("loop off: sent");
                    } else {
                        client.control("loop-one")?;
                        println!("loop track: sent");
                    }
                }
                Some(LoopCommand::Off { .. }) => {
                    client.control("loop-off")?;
                    println!("loop off: sent");
                }
                Some(LoopCommand::Track { .. }) => {
                    client.control("loop-one")?;
                    println!("loop track: sent");
                }
                Some(LoopCommand::Queue { .. }) => {
                    client.control("loop-queue")?;
                    println!("loop queue: sent");
                }
                Some(LoopCommand::Status { .. }) => {
                    println!("loop status: {:?}", client.loop_status()?);
                }
            }
            Ok(())
        }
        Command::Pair {
            address,
            code,
            node_name,
            name,
            select,
            no_select,
        } => match address {
            Some(address) => pair_peer(
                cli.config.as_deref(),
                &address,
                code,
                node_name,
                name,
                select,
                no_select,
            ),
            None => open_pairing().await,
        },
        Command::Device { command } => match command {
            DeviceCommand::List => {
                let devices = current_devices(cli.config.as_deref())?;
                let database = Database::open(default_db_path()?)?;
                print_devices(&devices, &database.authorized_clients()?);
                Ok(())
            }
            DeviceCommand::Add {
                name,
                address,
                token,
            } => {
                add_peer(cli.config.as_deref(), &name, &address, &token)?;
                println!("device added: {name}");
                Ok(())
            }
            DeviceCommand::Select { name } => {
                let devices = current_devices(cli.config.as_deref())?;
                let name = match name {
                    Some(name) => name,
                    None => selector::select_device(&devices)?,
                };
                let selected = set_selected_device(cli.config.as_deref(), &name)?;
                println!("selected device: {selected}");
                Ok(())
            }
            DeviceCommand::Remove { name } => {
                remove_peer(cli.config.as_deref(), &name)?;
                println!("device removed: {name}");
                Ok(())
            }
            DeviceCommand::Authorize { name } => {
                let database = Database::open(default_db_path()?)?;
                let token = authorize_client(&database, &name)?;
                println!("Authorized client \"{name}\".");
                println!();
                println!("Token:");
                println!("  {token}");
                println!();
                println!("This token is shown only once.");
                Ok(())
            }
            DeviceCommand::Revoke { name } => {
                let database = Database::open(default_db_path()?)?;
                if database.revoke_authorized_client(&name)? {
                    println!("revoked client: {name}");
                    Ok(())
                } else {
                    anyhow::bail!("unknown active authorized client `{name}`")
                }
            }
        },
        Command::Config { command } => match command {
            ConfigCommand::Init {
                force,
                peer_url,
                bind,
            } => {
                let path = Config::init(ConfigInit {
                    config_path: cli.config,
                    peer_url,
                    bind,
                    token: cli.token,
                    force,
                })?;
                println!("config: created {}", path.display());
                println!("config: token written but not printed");
                Ok(())
            }
        },
        Command::Token { command } => match command {
            TokenCommand::Generate => {
                println!("{}", generate_token()?);
                Ok(())
            }
        },
        Command::Status { to } => {
            let status = playback_client_for(cli.config, cli.peer_url, cli.token, to)?.status()?;
            print_status(&status);
            Ok(())
        }
        Command::History { to } => {
            let history =
                playback_client_for(cli.config, cli.peer_url, cli.token, to)?.history()?;
            print_history(&history);
            Ok(())
        }
    }
}

enum PlaybackClient {
    Node {
        client: NodeClient,
        to: Option<String>,
    },
    Http(PeerClient),
}

impl PlaybackClient {
    fn play(&self, url: &str) -> Result<()> {
        match self {
            Self::Node { client, to } => client.play(url, to.as_deref()),
            Self::Http(client) => client.play(url),
        }
    }

    fn queue(&self, url: &str) -> Result<()> {
        match self {
            Self::Node { client, to } => client.queue(url, to.as_deref()),
            Self::Http(client) => client.queue(url),
        }
    }

    fn control(&self, action: &str) -> Result<()> {
        match self {
            Self::Node { client, to } => client.control(action, to.as_deref()),
            Self::Http(client) => client.control(action),
        }
    }

    fn loop_status(&self) -> Result<LoopStatus> {
        match self {
            Self::Node { client, to } => client.loop_status(to.as_deref()),
            Self::Http(client) => client.loop_status(),
        }
    }

    fn status(&self) -> Result<MpvStatus> {
        match self {
            Self::Node { client, to } => client.status(to.as_deref()),
            Self::Http(client) => client.status(),
        }
    }

    fn history(&self) -> Result<Vec<HistoryEntry>> {
        match self {
            Self::Node { client, to } => client.history(to.as_deref()),
            Self::Http(client) => client.history(),
        }
    }
}

fn playback_client_for(
    config_path: Option<std::path::PathBuf>,
    peer_url: Option<String>,
    token: Option<String>,
    to: Option<String>,
) -> Result<PlaybackClient> {
    let has_legacy_override = peer_url.is_some()
        || token.is_some()
        || std::env::var_os("URA_PEER_URL").is_some()
        || std::env::var_os("URA_RECEIVER_URL").is_some()
        || std::env::var_os("URA_TOKEN").is_some();
    if has_legacy_override {
        let config = Config::load_with_overrides(config_path, peer_url, token)?;
        return Ok(PlaybackClient::Http(PeerClient::new(
            config.peer_url,
            config.token,
        )?));
    }

    if config_path.is_none() && NodeClient::is_available() {
        return Ok(PlaybackClient::Node {
            client: NodeClient::new()?,
            to,
        });
    }

    match resolve_destination(config_path.as_deref(), to.as_deref())? {
        Destination::Peer(peer) => Ok(PlaybackClient::Http(PeerClient::new(peer.url, peer.token)?)),
        Destination::SelfNode { name } => anyhow::bail!(
            "this device (`{name}`) is selected, but the local ura node is not running\n\nStart it with:\n  ura daemon"
        ),
    }
}

fn current_devices(config_path: Option<&std::path::Path>) -> Result<DeviceSet> {
    if config_path.is_none() && NodeClient::is_available() {
        NodeClient::new()?.devices()
    } else {
        device_set(config_path)
    }
}

fn set_selected_device(config_path: Option<&std::path::Path>, name: &str) -> Result<String> {
    if config_path.is_none() && NodeClient::is_available() {
        NodeClient::new()?.select(name)
    } else {
        select_device(config_path, name)
    }
}

async fn open_pairing() -> Result<()> {
    let started: PairControlResponse = control_socket_request("pair_start")?;
    let PairControlResponse::PairStart {
        code,
        expires_in,
        address,
    } = started
    else {
        match started {
            PairControlResponse::Error { error } => anyhow::bail!("{error}"),
            _ => anyhow::bail!("node returned an unexpected pairing response"),
        }
    };

    println!("ura pairing");
    println!();
    println!("address:");
    println!("  {address}");
    println!();
    println!("code:");
    println!("  {code}");
    println!();
    println!("expires in:");
    println!("  {expires_in} seconds");
    println!();
    println!("waiting for a device...");

    loop {
        tokio::select! {
            result = tokio::signal::ctrl_c() => {
                result.with_context(|| "failed to listen for Ctrl+C")?;
                let _ = control_socket_request::<PairControlResponse>("pair_cancel");
                println!("pairing cancelled");
                return Ok(());
            }
            _ = tokio::time::sleep(Duration::from_secs(1)) => {
                match control_socket_request::<PairControlResponse>("pair_status") {
                    Ok(PairControlResponse::PairStatus { completion, status }) => {
                        match completion {
                            PairingCompletion::Paired { device_name } => {
                                println!();
                                println!("paired device:");
                                println!("  {device_name}");
                                return Ok(());
                            }
                            PairingCompletion::Expired => {
                                println!("pairing expired");
                                return Ok(());
                            }
                            PairingCompletion::AttemptsExhausted => {
                                println!("pairing closed after too many invalid attempts");
                                return Ok(());
                            }
                            PairingCompletion::Cancelled | PairingCompletion::Inactive => {
                                println!("pairing closed");
                                return Ok(());
                            }
                            PairingCompletion::Active => {
                                if matches!(status, PairingStatus::Inactive) {
                                    println!("pairing expired");
                                    return Ok(());
                                }
                            }
                        }
                    }
                    Ok(PairControlResponse::Error { error }) => anyhow::bail!("{error}"),
                    Ok(_) => anyhow::bail!("node returned an unexpected pairing status response"),
                    Err(error) => anyhow::bail!("node became unavailable: {error:#}"),
                }
            }
        }
    }
}

fn pair_peer(
    config_path: Option<&std::path::Path>,
    address: &str,
    code: Option<String>,
    node_name: Option<String>,
    name: Option<String>,
    select: bool,
    no_select: bool,
) -> Result<()> {
    let normalized_address = normalize_peer_address(address)?;
    let pairing_client = PeerPairingClient::new(normalized_address.clone())?;
    let info = pairing_client.info()?;
    let existing_config = DeviceConfig::load(config_path)?;
    let tty = io::stdin().is_terminal();
    let default_local_name = existing_config
        .local_name
        .clone()
        .unwrap_or_else(default_device_name);
    let default_peer_alias = info.node_name.clone();

    let code = match code {
        Some(code) => code,
        None if tty => prompt("pairing code", None)?,
        None => anyhow::bail!("--code is required when stdin is not a TTY"),
    };
    if !valid_pairing_code(&code) {
        anyhow::bail!("pairing code must be six decimal digits");
    }

    let local_name = match node_name {
        Some(name) => name,
        None if existing_config.local_name.is_some() => default_local_name.clone(),
        None if tty => prompt("this device name", Some(&default_local_name))?,
        None => anyhow::bail!("--device-name is required when stdin is not a TTY"),
    };
    let peer_alias = match name {
        Some(name) => name,
        None if tty => prompt("save peer as", Some(&default_peer_alias))?,
        None => anyhow::bail!("--name is required when stdin is not a TTY"),
    };
    if peer_alias == local_name {
        anyhow::bail!("peer name `{peer_alias}` conflicts with this device");
    }

    if existing_config
        .devices
        .iter()
        .any(|device| device.name == peer_alias)
    {
        anyhow::bail!("device `{peer_alias}` already exists");
    }

    let should_select = if select {
        true
    } else if no_select {
        false
    } else if tty {
        prompt_yes_no("select this device?", true)?
    } else {
        anyhow::bail!("--select or --no-select is required when stdin is not a TTY");
    };

    let claim = pairing_client.claim(&code, &local_name)?;
    if claim.protocol_version != 1 {
        anyhow::bail!(
            "peer returned unsupported pairing protocol {}",
            claim.protocol_version
        );
    }

    match add_paired_peer(
        config_path,
        &peer_alias,
        &normalized_address,
        &claim.token,
        &local_name,
        should_select,
    ) {
        Ok(()) => {
            println!("paired with \"{peer_alias}\"");
            if should_select {
                println!("selected device: {peer_alias}");
            }
            Ok(())
        }
        Err(error) => {
            println!("pairing succeeded on the peer, but local config could not be saved");
            println!();
            println!("token:");
            println!("  {}", claim.token);
            Err(error)
        }
    }
}

fn prompt(label: &str, default: Option<&str>) -> Result<String> {
    match default {
        Some(default) => print!("{label} [{default}]: "),
        None => print!("{label}: "),
    }
    io::stdout().flush()?;
    let mut value = String::new();
    io::stdin().read_line(&mut value)?;
    let value = value.trim();
    if value.is_empty() {
        default
            .map(str::to_string)
            .ok_or_else(|| anyhow::anyhow!("{label} is required"))
    } else {
        Ok(value.to_string())
    }
}

fn prompt_yes_no(label: &str, default_yes: bool) -> Result<bool> {
    let suffix = if default_yes { "[Y/n]" } else { "[y/N]" };
    print!("{label} {suffix}: ");
    io::stdout().flush()?;
    let mut value = String::new();
    io::stdin().read_line(&mut value)?;
    let value = value.trim().to_ascii_lowercase();
    if value.is_empty() {
        return Ok(default_yes);
    }
    match value.as_str() {
        "y" | "yes" => Ok(true),
        "n" | "no" => Ok(false),
        _ => anyhow::bail!("expected yes or no"),
    }
}

fn control_socket_request<T>(command: &str) -> Result<T>
where
    T: for<'de> Deserialize<'de>,
{
    let socket_path = default_control_socket_path()?;
    let mut stream = UnixStream::connect(&socket_path).with_context(|| {
        format!(
            "ura daemon is not running or pairing socket is unavailable at {}",
            socket_path.display()
        )
    })?;
    let request = serde_json::to_vec(&PairControlRequest { command })?;
    stream.write_all(&request)?;
    stream.shutdown(Shutdown::Write)?;

    let mut response = String::new();
    stream.read_to_string(&mut response)?;
    serde_json::from_str(&response).with_context(|| "failed to parse pairing response")
}

#[derive(Debug, Serialize)]
struct PairControlRequest<'a> {
    command: &'a str,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum PairControlResponse {
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
        #[serde(rename = "ok")]
        _ok: bool,
    },
    Error {
        error: String,
    },
}

fn command_to_device(parent_to: &Option<String>, command: &Option<LoopCommand>) -> Option<String> {
    match command {
        Some(LoopCommand::Off { to })
        | Some(LoopCommand::Track { to })
        | Some(LoopCommand::Queue { to })
        | Some(LoopCommand::Status { to }) => to.clone().or_else(|| parent_to.clone()),
        None => parent_to.clone(),
    }
}

fn print_status(status: &MpvStatus) {
    if (status.idle_active.unwrap_or(false)
        || (status.pause.is_none()
            && status.path.is_none()
            && status.source_url.is_none()
            && status.title.is_none()
            && status.media_title.is_none()))
        && status.title.is_none()
        && status.media_title.is_none()
    {
        println!("idle");
        return;
    }

    println!(
        "{}",
        if status.pause.unwrap_or(false) {
            "paused"
        } else {
            "playing"
        }
    );
    println!("title:   {}", display_title(status));
    if let Some(artist) = status.artist.as_deref().or(status.uploader.as_deref()) {
        println!("artist:  {artist}");
    }
    println!(
        "time:    {} / {}",
        format_position(status.position_seconds),
        format_duration(status.duration_seconds)
    );
    if let (Some(position), Some(count)) = (status.playlist_pos, status.playlist_count) {
        println!("queue:   {} / {}", position + 1, count);
    }
    if let Some(loop_status) = &status.loop_status {
        println!("loop:    {}", format_loop_status(loop_status));
    }
    if status.title.is_none()
        && status.media_title.is_none()
        && let Some(source_url) = &status.source_url
    {
        println!("source:  {}", shorten_source(source_url));
    }
}

fn print_history(history: &[HistoryEntry]) {
    println!("{:<20}  {:<28}  SOURCE", "TIMESTAMP", "TITLE");
    for entry in history {
        let timestamp = entry
            .display_played_at
            .as_deref()
            .or(entry.last_played_at.as_deref())
            .unwrap_or(entry.created_at.as_str());
        let title = entry.title.as_deref().unwrap_or("Unknown title");
        println!(
            "{:<20}  {:<28}  {}",
            timestamp,
            truncate(title, 28),
            entry.source_kind
        );
    }
}

fn print_devices(devices: &DeviceSet, authorized: &[ura::db::AuthorizedClient]) {
    print!("{}", render_devices(devices, authorized));
}

fn render_devices(devices: &DeviceSet, authorized: &[ura::db::AuthorizedClient]) -> String {
    let mut output = String::new();
    output.push_str(&format!("Selected device: {}\n\n", devices.selected));
    output.push_str("Devices:\n");
    output.push_str(&format!(
        "  {:<2} {:<18} {:<12} ADDRESS\n",
        "", "NAME", "TYPE"
    ));
    for device in &devices.devices {
        let marker = if device.name == devices.selected {
            "*"
        } else {
            " "
        };
        let kind = match &device.kind {
            DeviceKind::ThisDevice => "this device",
            DeviceKind::Peer => "peer",
        };
        output.push_str(&format!(
            "  {:<2} {:<18} {:<12} {}\n",
            marker,
            device.name,
            kind,
            device.address.as_deref().unwrap_or("-")
        ));
    }

    output.push('\n');
    output.push_str("Authorized clients:\n");
    if authorized.is_empty() {
        output.push_str("  none\n");
    } else {
        output.push_str(&format!("  {:<18} {:<16} LAST SEEN\n", "NAME", "CREATED"));
        for device in authorized {
            output.push_str(&format!(
                "  {:<18} {:<16} {}\n",
                device.name,
                device.created_at,
                device.last_seen_at.as_deref().unwrap_or("never")
            ));
        }
    }
    output
}

fn display_title(status: &MpvStatus) -> String {
    status
        .title
        .as_deref()
        .or(status.media_title.as_deref())
        .filter(|title| status.source_url.as_deref().is_none_or(|url| *title != url))
        .map(str::to_string)
        .unwrap_or_else(|| "Unknown title".to_string())
}

fn format_duration(duration: Option<f64>) -> String {
    match duration {
        Some(duration) if duration.is_finite() && duration > 0.0 => {
            format_seconds(duration.round() as u64)
        }
        Some(duration) if duration.is_infinite() => "live".to_string(),
        _ => "unknown".to_string(),
    }
}

fn format_position(position: Option<f64>) -> String {
    match position {
        Some(position) if position.is_finite() && position >= 0.0 => {
            format_seconds(position.round() as u64)
        }
        _ => "unknown".to_string(),
    }
}

fn format_seconds(seconds: u64) -> String {
    let hours = seconds / 3600;
    let minutes = (seconds % 3600) / 60;
    let seconds = seconds % 60;
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

fn format_loop_status(status: &LoopStatus) -> &'static str {
    match status {
        LoopStatus::Off => "off",
        LoopStatus::One => "track",
        LoopStatus::Queue => "queue",
        LoopStatus::Custom { .. } => "custom",
    }
}

fn shorten_source(source: &str) -> String {
    source
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(source)
        .chars()
        .take(48)
        .collect()
}

fn truncate(value: &str, width: usize) -> String {
    if value.chars().count() <= width {
        return value.to_string();
    }
    if width <= 3 {
        return ".".repeat(width);
    }
    let prefix = value.chars().take(width - 3).collect::<String>();
    format!("{prefix}...")
}

fn init_daemon_logging() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("ura=info"));
    let _ = tracing_subscriber::fmt().with_env_filter(filter).try_init();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_duration_without_null_or_zero_for_missing_values() {
        assert_eq!(format_duration(None), "unknown");
        assert_eq!(format_duration(Some(222.5)), "3:43");
        assert_eq!(format_duration(Some(3798.0)), "1:03:18");
    }

    #[test]
    fn status_title_uses_unknown_instead_of_source_url() {
        let status = MpvStatus {
            pause: Some(false),
            idle_active: Some(false),
            path: None,
            media_title: Some("https://youtu.be/example".to_string()),
            title: None,
            artist: None,
            uploader: None,
            album: None,
            duration_seconds: None,
            position_seconds: None,
            playlist_pos: None,
            playlist_count: None,
            source_url: Some("https://youtu.be/example".to_string()),
            playback_path: None,
            loop_status: Some(LoopStatus::Off),
        };

        assert_eq!(display_title(&status), "Unknown title");
    }

    #[test]
    fn truncate_uses_ascii_suffix() {
        assert_eq!(truncate("abcdef", 6), "abcdef");
        assert_eq!(truncate("abcdef", 5), "ab...");
        assert_eq!(truncate("abcdef", 3), "...");
        assert_eq!(truncate("abcdef", 2), "..");
    }

    #[test]
    fn truncate_counts_unicode_characters() {
        assert_eq!(truncate("あいうえお", 4), "あ...");
        assert_eq!(truncate("あいう", 3), "あいう");
    }

    #[test]
    fn device_list_rendering_hides_token_material() {
        let devices = DeviceSet {
            selected: "kamo".to_string(),
            devices: vec![
                ura::node::DeviceSummary {
                    name: "desktop".to_string(),
                    kind: DeviceKind::ThisDevice,
                    address: None,
                },
                ura::node::DeviceSummary {
                    name: "kamo".to_string(),
                    kind: DeviceKind::Peer,
                    address: Some("http://kamo:8765".to_string()),
                },
            ],
        };
        let authorized = vec![ura::db::AuthorizedClient {
            name: "firefox".to_string(),
            created_at: "2026-07-10 17:35".to_string(),
            last_seen_at: None,
            revoked_at: None,
        }];

        let output = render_devices(&devices, &authorized);

        assert!(output.contains("kamo"));
        assert!(output.contains("firefox"));
        assert!(!output.contains("secret-token"));
        assert!(!output.contains("token_hash"));
    }
}
