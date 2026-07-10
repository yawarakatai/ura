use std::{net::SocketAddr, path::PathBuf};

use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "ura", version, about = "Tiny LAN/Tailscale audio caster")]
pub struct Cli {
    #[arg(long, global = true)]
    pub config: Option<PathBuf>,

    #[arg(long, global = true)]
    pub receiver_url: Option<String>,

    #[arg(long, global = true)]
    pub token: Option<String>,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Start the long-running local audio receiver.
    Serve {
        /// Address for the HTTP receiver API to bind.
        #[arg(long)]
        bind: Option<SocketAddr>,
    },
    /// Play a URL immediately.
    Play { url: String },
    /// Add a media URL to the end of the selected device's playback queue.
    Queue { url: String },
    /// Pause playback on the receiver.
    Pause,
    /// Resume playback on the receiver.
    Resume,
    /// Toggle pause/resume on the receiver.
    Toggle,
    /// Stop playback on the receiver.
    Stop,
    /// Control playback looping.
    Loop {
        #[command(subcommand)]
        command: Option<LoopCommand>,
    },
    /// Manage local ura configuration.
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
    /// Manage receiver tokens.
    Token {
        #[command(subcommand)]
        command: TokenCommand,
    },
    /// Show current receiver status.
    Status,
    /// Show playback history.
    History,
}

#[derive(Debug, Subcommand)]
pub enum LoopCommand {
    /// Disable current-track and queue looping.
    Off,
    /// Loop the current track forever.
    Track,
    /// Loop the playback queue forever.
    Queue,
    /// Show the current loop mode.
    Status,
}

#[derive(Debug, Subcommand)]
pub enum ConfigCommand {
    /// Create the main ura config file.
    Init {
        /// Overwrite an existing config file.
        #[arg(long)]
        force: bool,

        /// Receiver URL to write into config.toml.
        #[arg(long, default_value = "http://127.0.0.1:8765")]
        receiver_url: String,

        /// Bind address to write into config.toml for ura serve.
        #[arg(long, default_value = "127.0.0.1:8765")]
        bind: SocketAddr,
    },
}

#[derive(Debug, Subcommand)]
pub enum TokenCommand {
    /// Print a new random receiver token.
    Generate,
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory;

    use super::*;

    #[test]
    fn parses_current_public_commands() {
        for args in [
            ["ura", "serve"].as_slice(),
            ["ura", "play", "https://youtu.be/example"].as_slice(),
            ["ura", "queue", "https://youtu.be/example"].as_slice(),
            ["ura", "pause"].as_slice(),
            ["ura", "resume"].as_slice(),
            ["ura", "toggle"].as_slice(),
            ["ura", "stop"].as_slice(),
            ["ura", "status"].as_slice(),
            ["ura", "history"].as_slice(),
            ["ura", "loop"].as_slice(),
            ["ura", "loop", "off"].as_slice(),
            ["ura", "loop", "track"].as_slice(),
            ["ura", "loop", "queue"].as_slice(),
            ["ura", "loop", "status"].as_slice(),
        ] {
            Cli::try_parse_from(args).expect("current command should parse");
        }
    }

    #[test]
    fn rejects_removed_public_commands() {
        for args in [
            ["ura", "receive"].as_slice(),
            ["ura", "enqueue", "https://youtu.be/example"].as_slice(),
        ] {
            Cli::try_parse_from(args).expect_err("removed command should be rejected");
        }
    }

    #[test]
    fn help_advertises_current_command_names_only() {
        let command = Cli::command();
        let subcommands = command
            .get_subcommands()
            .map(|command| command.get_name().to_string())
            .collect::<Vec<_>>();
        for expected in ["serve", "queue", "pause", "resume"] {
            assert!(
                subcommands.contains(&expected.to_string()),
                "help should contain {expected}"
            );
        }
        for removed in ["receive", "enqueue"] {
            assert!(
                !subcommands.contains(&removed.to_string()),
                "help should not advertise {removed}"
            );
        }

        let mut command = Cli::command();
        let mut help = Vec::new();
        command.write_long_help(&mut help).expect("write help");
        let help = String::from_utf8(help).expect("help should be UTF-8");
        assert!(help.contains("Add a media URL to the end"));
        assert!(!help.contains("Add a URL to the playback queue"));
    }
}
