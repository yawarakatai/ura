use anyhow::Result;
use clap::Parser;
use ura::{
    cli::{Cli, Command, ConfigCommand, DeviceCommand, LoopCommand, TokenCommand},
    client::HttpClient,
    config::{Config, ConfigInit, DeviceConfig, ReceiverConfig, generate_token},
    db::{Database, HistoryEntry},
    mpv::{LoopStatus, MpvStatus},
    receiver::run_serve,
};

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Serve { bind } => {
            init_serve_logging();
            let config = ReceiverConfig::load_with_overrides(cli.config, bind, cli.token)?;
            run_serve(config.bind, config.token).await
        }
        Command::Play { to, url } => {
            let client = client_for(cli.config, cli.receiver_url, cli.token, to)?;
            client.play(&url)?;
            println!("play: sent {url}");
            Ok(())
        }
        Command::Queue { to, url } => {
            let client = client_for(cli.config, cli.receiver_url, cli.token, to)?;
            client.queue(&url)?;
            println!("queue: sent {url}");
            Ok(())
        }
        Command::Pause { to } => {
            let client = client_for(cli.config, cli.receiver_url, cli.token, to)?;
            client.control("pause")?;
            println!("pause: sent");
            Ok(())
        }
        Command::Resume { to } => {
            let client = client_for(cli.config, cli.receiver_url, cli.token, to)?;
            client.control("resume")?;
            println!("resume: sent");
            Ok(())
        }
        Command::Toggle { to } => {
            let client = client_for(cli.config, cli.receiver_url, cli.token, to)?;
            client.control("toggle")?;
            println!("toggle: sent");
            Ok(())
        }
        Command::Stop { to } => {
            let client = client_for(cli.config, cli.receiver_url, cli.token, to)?;
            client.control("stop")?;
            println!("stop: sent");
            Ok(())
        }
        Command::Loop { to, command } => {
            let to = command_to_device(&to, &command);
            let client = client_for(cli.config, cli.receiver_url, cli.token, to)?;
            match command {
                None => {
                    let status = client.loop_status()?;
                    if status == LoopStatus::One {
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
                    let status = client.loop_status()?;
                    println!("loop status: {status:?}");
                }
            }
            Ok(())
        }
        Command::Config { command } => match command {
            ConfigCommand::Init {
                force,
                receiver_url,
                bind,
            } => {
                let path = Config::init(ConfigInit {
                    config_path: cli.config,
                    receiver_url,
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
        Command::Device { command } => match command {
            DeviceCommand::List => {
                let devices = DeviceConfig::load(cli.config.as_deref())?;
                let database = Database::open(ura::config::default_db_path()?)?;
                let authorized = database.authorized_devices()?;
                print_devices(&devices, &authorized);
                Ok(())
            }
            DeviceCommand::Add {
                name,
                address,
                token,
            } => {
                DeviceConfig::add(cli.config.as_deref(), &name, &address, &token)?;
                println!("device added: {name}");
                Ok(())
            }
            DeviceCommand::Select { name } => {
                DeviceConfig::select(cli.config.as_deref(), &name)?;
                println!("selected device: {name}");
                Ok(())
            }
            DeviceCommand::Remove { name } => {
                DeviceConfig::remove(cli.config.as_deref(), &name)?;
                println!("device removed: {name}");
                Ok(())
            }
            DeviceCommand::Authorize { name } => {
                let token = generate_token()?;
                let database = Database::open(ura::config::default_db_path()?)?;
                database.authorize_device(&name, &token)?;
                println!("Authorized device \"{name}\".");
                println!();
                println!("Token:");
                println!("  {token}");
                println!();
                println!("This token is shown only once.");
                println!("Store it on the controlling device.");
                Ok(())
            }
            DeviceCommand::Revoke { name } => {
                let database = Database::open(ura::config::default_db_path()?)?;
                if database.revoke_authorized_device(&name)? {
                    println!("revoked device: {name}");
                    Ok(())
                } else {
                    anyhow::bail!("unknown active authorized device `{name}`")
                }
            }
        },
        Command::Status { to } => {
            let client = client_for(cli.config, cli.receiver_url, cli.token, to)?;
            let status = client.status()?;
            print_status(&status);
            Ok(())
        }
        Command::History { to } => {
            let client = client_for(cli.config, cli.receiver_url, cli.token, to)?;
            let history = client.history()?;
            print_history(&history);
            Ok(())
        }
    }
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

fn client_for(
    config_path: Option<std::path::PathBuf>,
    receiver_url: Option<String>,
    token: Option<String>,
    to: Option<String>,
) -> Result<HttpClient> {
    let config = if receiver_url.is_some() || token.is_some() {
        Config::load_with_overrides(config_path, receiver_url, token)?
    } else {
        Config::load_for_device(config_path, to)?
    };
    HttpClient::new(config.receiver_url, config.token)
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

    if status.pause.unwrap_or(false) {
        println!("paused");
    } else {
        println!("playing");
    }

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
            .last_played_at
            .as_deref()
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

fn print_devices(config: &DeviceConfig, authorized: &[ura::db::AuthorizedDevice]) {
    print!("{}", render_devices(config, authorized));
}

fn render_devices(config: &DeviceConfig, authorized: &[ura::db::AuthorizedDevice]) -> String {
    let mut output = String::new();
    output.push_str(&format!(
        "Selected device: {}\n\n",
        config.selected_device.as_deref().unwrap_or("none")
    ));
    output.push_str("Remote devices:\n");
    if config.devices.is_empty() {
        if let Some(legacy) = &config.legacy_device {
            output.push_str(&format!("  {:<12}  {} (legacy)\n", legacy.name, legacy.url));
        } else {
            output.push_str("  none\n");
        }
    } else {
        output.push_str(&format!("  {:<12}  ADDRESS\n", "NAME"));
        for device in &config.devices {
            output.push_str(&format!("  {:<12}  {}\n", device.name, device.url));
        }
    }
    output.push('\n');
    output.push_str("Authorized devices:\n");
    if authorized.is_empty() {
        output.push_str("  none\n");
    } else {
        output.push_str(&format!("  {:<12}  {:<16}  LAST SEEN\n", "NAME", "CREATED"));
        for device in authorized {
            output.push_str(&format!(
                "  {:<12}  {:<16}  {}\n",
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
    let char_count = value.chars().count();
    if char_count <= width {
        return value.to_string();
    }
    if width <= 3 {
        return ".".repeat(width);
    }
    let prefix_width = width - 3;
    let mut truncated = value.chars().take(prefix_width).collect::<String>();
    truncated.push_str("...");
    truncated
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
        let config = DeviceConfig {
            selected_device: Some("kamo".to_string()),
            devices: vec![ura::config::RemoteDevice {
                name: "kamo".to_string(),
                url: "http://kamo:8765".to_string(),
                token: "secret-token".to_string(),
            }],
            legacy_device: None,
        };
        let authorized = vec![ura::db::AuthorizedDevice {
            name: "firefox".to_string(),
            created_at: "2026-07-10 17:35".to_string(),
            last_seen_at: None,
            revoked_at: None,
        }];

        let output = render_devices(&config, &authorized);

        assert!(output.contains("kamo"));
        assert!(output.contains("firefox"));
        assert!(!output.contains("secret-token"));
        assert!(!output.contains("token_hash"));
    }
}

fn init_serve_logging() {
    use tracing_subscriber::{EnvFilter, fmt, prelude::*};

    let env_filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("ura=info"));

    tracing_subscriber::registry()
        .with(env_filter)
        .with(fmt::layer())
        .init();
}
