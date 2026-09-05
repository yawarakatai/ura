use std::{
    io::{self, IsTerminal, Read, Write},
    net::Shutdown,
    os::unix::net::UnixStream,
    time::Duration,
};

use anyhow::{Context, Result};
use clap::Parser;
use serde::{Deserialize, Serialize};
use unicode_width::UnicodeWidthStr;
use ura::{
    cli::{Cli, Command, DeviceCommand, HistoryCommand},
    client::PeerPairingClient,
    config::{
        DeviceConfig, NodeConfig, default_control_socket_path, default_db_path,
        default_device_name, normalize_peer_address,
    },
    db::{Database, HistoryEntry, authorize_client},
    mpv::{LoopStatus, MpvStatus},
    node::{
        DeviceKind, DeviceSet, NodeClient, add_paired_peer, add_peer, device_set, remove_peer,
        run_node, select_device,
    },
    pairing::{PairingCompletion, PairingStatus, valid_pairing_code},
    selector,
};

#[tokio::main]
async fn main() -> Result<()> {
    let Cli {
        url,
        queue,
        loop_track,
        command,
    } = Cli::parse();

    match command {
        None => run_default(url, queue, loop_track),
        Some(Command::Toggle) => {
            NodeClient::new()?.control("toggle")?;
            println!("toggle: sent");
            Ok(())
        }
        Some(Command::Stop) => {
            NodeClient::new()?.control("stop")?;
            println!("stop: sent");
            Ok(())
        }
        Some(Command::History { command }) => run_history_command(command),
        Some(Command::Device { command }) => run_device_command(command),
        Some(Command::Pair {
            address,
            code,
            node_name,
            name,
            select,
            no_select,
        }) => match address {
            Some(address) => pair_peer(&address, code, node_name, name, select, no_select),
            None => open_pairing().await,
        },
        Some(Command::Daemon { bind }) => {
            init_daemon_logging();
            let config = NodeConfig::load(bind)?;
            run_node(config.bind, None).await
        }
    }
}

fn run_default(url: Option<String>, queue: bool, loop_track: bool) -> Result<()> {
    let client = NodeClient::new()?;
    match url {
        Some(url) if queue => {
            client.queue(&url)?;
            println!("queue: sent {url}");
        }
        Some(url) => {
            client.play_with_loop(&url, loop_track)?;
            println!("play: sent {url}");
        }
        None => print_status(&client.status()?),
    }
    Ok(())
}

fn run_history_command(command: Option<HistoryCommand>) -> Result<()> {
    let client = NodeClient::new()?;
    let history = client.history()?;
    match command {
        None | Some(HistoryCommand::List) => print_history(&history),
        Some(HistoryCommand::Replay { index }) => {
            let entry = history_entry(&history, index.get())?;
            let title = entry.title.as_deref().unwrap_or("Unknown title");
            client.play(&entry.source_url)?;
            println!("history replay #{}: sent {title}", index.get());
        }
    }
    Ok(())
}

fn history_entry(history: &[HistoryEntry], index: usize) -> Result<&HistoryEntry> {
    if history.is_empty() {
        anyhow::bail!("playback history is empty");
    }
    if index == 0 {
        anyhow::bail!("history entry numbers start at 1");
    }
    history.get(index - 1).ok_or_else(|| {
        anyhow::anyhow!(
            "history entry {index} does not exist; choose an entry from 1 to {}",
            history.len()
        )
    })
}

fn run_device_command(command: DeviceCommand) -> Result<()> {
    match command {
        DeviceCommand::List => {
            let devices = current_devices()?;
            let database = Database::open(default_db_path()?)?;
            print_devices(&devices, &database.authorized_clients()?);
        }
        DeviceCommand::Add {
            name,
            address,
            token,
        } => {
            add_peer(None, &name, &address, &token)?;
            println!("device added: {name}");
        }
        DeviceCommand::Select { name } => {
            let devices = current_devices()?;
            let name = match name {
                Some(name) => name,
                None => selector::select_device(&devices)?,
            };
            let selected = set_selected_device(&name)?;
            println!("selected device: {selected}");
        }
        DeviceCommand::Remove { name } => {
            remove_peer(None, &name)?;
            println!("device removed: {name}");
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
        }
        DeviceCommand::Revoke { name } => {
            let database = Database::open(default_db_path()?)?;
            if database.revoke_authorized_client(&name)? {
                println!("revoked client: {name}");
            } else {
                anyhow::bail!("unknown active authorized client `{name}`");
            }
        }
    }
    Ok(())
}

fn current_devices() -> Result<DeviceSet> {
    if NodeClient::is_available() {
        NodeClient::new()?.devices()
    } else {
        device_set(None)
    }
}

fn set_selected_device(name: &str) -> Result<String> {
    if NodeClient::is_available() {
        NodeClient::new()?.select(name)
    } else {
        select_device(None, name)
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
    let existing_config = DeviceConfig::load(None)?;
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
        None,
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
    print!("{}", render_history(history));
}

fn render_history(history: &[HistoryEntry]) -> String {
    let mut output = format!(
        "{:>4}  {:<20}  {}  SOURCE\n",
        "#",
        "TIMESTAMP",
        format_column("TITLE", 28)
    );
    for (index, entry) in history.iter().enumerate() {
        let timestamp = entry
            .display_played_at
            .as_deref()
            .or(entry.last_played_at.as_deref())
            .unwrap_or(entry.created_at.as_str());
        let title = entry.title.as_deref().unwrap_or("Unknown title");
        output.push_str(&format!(
            "{:>4}  {:<20}  {}  {}\n",
            index + 1,
            timestamp,
            format_column(title, 28),
            entry.source_kind
        ));
    }
    output
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

fn format_column(value: &str, width: usize) -> String {
    let mut value = truncate(value, width);
    let padding = width.saturating_sub(UnicodeWidthStr::width(value.as_str()));
    value.push_str(&" ".repeat(padding));
    value
}

fn truncate(value: &str, width: usize) -> String {
    if UnicodeWidthStr::width(value) <= width {
        return value.to_string();
    }
    if width <= 3 {
        return ".".repeat(width);
    }

    let prefix_width = width - 3;
    let mut prefix_end = 0;
    for (index, character) in value.char_indices() {
        let end = index + character.len_utf8();
        if UnicodeWidthStr::width(&value[..end]) <= prefix_width {
            prefix_end = end;
        }
    }
    format!("{}...", &value[..prefix_end])
}

fn init_daemon_logging() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("ura=info"));
    let _ = tracing_subscriber::fmt().with_env_filter(filter).try_init();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn history_fixture(title: &str, source_url: &str) -> HistoryEntry {
        HistoryEntry {
            source_kind: "url".to_string(),
            source_url: source_url.to_string(),
            play_url: source_url.to_string(),
            title: Some(title.to_string()),
            uploader: None,
            duration: None,
            thumbnail_url: None,
            created_at: "1".to_string(),
            last_played_at: Some("1".to_string()),
            display_played_at: Some("1970-01-01 00:00:01".to_string()),
            play_count: 1,
        }
    }

    #[test]
    fn history_entries_use_latest_first_one_based_numbers() {
        let history = vec![
            history_fixture("Latest", "https://youtu.be/latest"),
            history_fixture("Earlier", "https://youtu.be/earlier"),
        ];

        assert_eq!(
            history_entry(&history, 2)
                .expect("second history entry")
                .source_url,
            "https://youtu.be/earlier"
        );
        assert!(history_entry(&history, 0).is_err());
        assert!(history_entry(&history, 3).is_err());

        let rendered = render_history(&history);
        assert!(rendered.contains("   1  1970-01-01 00:00:01   Latest"));
        assert!(rendered.contains("   2  1970-01-01 00:00:01   Earlier"));
    }

    #[test]
    fn empty_history_cannot_be_replayed() {
        let error = history_entry(&[], 1).expect_err("empty history should fail");

        assert!(error.to_string().contains("history is empty"));
    }

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
    fn truncate_uses_terminal_display_width() {
        assert_eq!(truncate("abcあいう", 8), "abcあ...");
        assert_eq!(truncate("あいう", 6), "あいう");
        assert_eq!(truncate("e\u{301}fg", 3), "e\u{301}fg");
    }

    #[test]
    fn format_column_pads_to_terminal_display_width() {
        let column = format_column("あいうえお", 8);

        assert_eq!(column, "あい... ");
        assert_eq!(UnicodeWidthStr::width(column.as_str()), 8);
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
