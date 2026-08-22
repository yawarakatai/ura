use std::{net::SocketAddr, path::PathBuf};

use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    name = "ura",
    version,
    about = "Play audio on this or another paired Linux device."
)]
pub struct Cli {
    #[arg(long, global = true)]
    pub config: Option<PathBuf>,

    #[arg(long = "peer-url", alias = "receiver-url", global = true)]
    pub receiver_url: Option<String>,

    #[arg(long, global = true)]
    pub token: Option<String>,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Run the long-running local ura node.
    #[command(name = "daemon", alias = "serve")]
    Serve {
        /// Address for the peer HTTP API to bind.
        #[arg(long)]
        bind: Option<SocketAddr>,
    },
    /// Play a URL immediately on the selected device.
    Play {
        /// Override the selected device for this command only.
        #[arg(long)]
        to: Option<String>,
        url: String,
    },
    /// Add a media URL to the selected device's playback queue.
    Queue {
        /// Override the selected device for this command only.
        #[arg(long)]
        to: Option<String>,
        url: String,
    },
    /// Pause playback on the selected device.
    Pause {
        #[arg(long)]
        to: Option<String>,
    },
    /// Resume playback on the selected device.
    Resume {
        #[arg(long)]
        to: Option<String>,
    },
    /// Toggle pause/resume on the selected device.
    Toggle {
        #[arg(long)]
        to: Option<String>,
    },
    /// Stop playback on the selected device.
    Stop {
        #[arg(long)]
        to: Option<String>,
    },
    /// Control playback looping.
    Loop {
        #[arg(long)]
        to: Option<String>,
        #[command(subcommand)]
        command: Option<LoopCommand>,
    },
    /// Open pairing on this node, or pair with another node by address.
    Pair {
        /// Peer address to pair with. Omit to open pairing on this node.
        address: Option<String>,
        /// Six-digit pairing code shown on the peer.
        #[arg(long)]
        code: Option<String>,
        /// This node's name stored on the peer.
        #[arg(long)]
        device_name: Option<String>,
        /// Local alias for the peer being added.
        #[arg(long)]
        name: Option<String>,
        /// Select the newly paired peer without prompting.
        #[arg(long, conflicts_with = "no_select")]
        select: bool,
        /// Do not select the newly paired peer without prompting.
        #[arg(long, conflicts_with = "select")]
        no_select: bool,
    },
    /// Manage playback devices and peer credentials.
    Device {
        #[command(subcommand)]
        command: DeviceCommand,
    },
    /// Manage local ura configuration.
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
    /// Manage legacy peer API tokens.
    Token {
        #[command(subcommand)]
        command: TokenCommand,
    },
    /// Show current playback status for the selected device.
    Status {
        #[arg(long)]
        to: Option<String>,
    },
    /// Show playback history for the selected device.
    History {
        #[arg(long)]
        to: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
pub enum LoopCommand {
    /// Disable current-track and queue looping.
    Off {
        #[arg(long)]
        to: Option<String>,
    },
    /// Loop the current track forever.
    Track {
        #[arg(long)]
        to: Option<String>,
    },
    /// Loop the playback queue forever.
    Queue {
        #[arg(long)]
        to: Option<String>,
    },
    /// Show the current loop mode.
    Status {
        #[arg(long)]
        to: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
pub enum ConfigCommand {
    /// Create the main ura config file.
    Init {
        /// Overwrite an existing config file.
        #[arg(long)]
        force: bool,

        /// Peer API URL to write into config.toml.
        #[arg(
            long = "peer-url",
            alias = "receiver-url",
            default_value = "http://127.0.0.1:8765"
        )]
        receiver_url: String,

        /// Peer API bind address to write into config.toml for `ura daemon`.
        #[arg(long, default_value = "127.0.0.1:8765")]
        bind: SocketAddr,
    },
}

#[derive(Debug, Subcommand)]
pub enum TokenCommand {
    /// Print a new random legacy peer API token.
    Generate,
}

#[derive(Debug, Subcommand)]
pub enum DeviceCommand {
    /// List this device, configured peers, and authorized clients.
    List,
    /// Add a peer this node can control.
    Add {
        name: String,
        address: String,
        #[arg(long)]
        token: String,
    },
    /// Select the default playback device. Omit NAME for an interactive picker.
    Select { name: Option<String> },
    /// Remove a peer from local configuration.
    Remove { name: String },
    /// Authorize a client on this node.
    Authorize { name: String },
    /// Revoke a client credential on this node.
    Revoke { name: String },
}

#[cfg(test)]
mod tests {
    use clap::{CommandFactory, Parser};

    use super::*;

    #[test]
    fn parses_current_public_commands() {
        for args in [
            ["ura", "daemon"].as_slice(),
            ["ura", "play", "https://youtu.be/example"].as_slice(),
            ["ura", "play", "--to", "kamo", "https://youtu.be/example"].as_slice(),
            ["ura", "queue", "https://youtu.be/example"].as_slice(),
            ["ura", "pause"].as_slice(),
            ["ura", "resume"].as_slice(),
            ["ura", "toggle"].as_slice(),
            ["ura", "stop"].as_slice(),
            ["ura", "status"].as_slice(),
            ["ura", "status", "--to", "kamo"].as_slice(),
            ["ura", "history"].as_slice(),
            ["ura", "history", "--to", "kamo"].as_slice(),
            ["ura", "loop"].as_slice(),
            ["ura", "loop", "off"].as_slice(),
            ["ura", "loop", "off", "--to", "kamo"].as_slice(),
            ["ura", "loop", "track"].as_slice(),
            ["ura", "loop", "queue"].as_slice(),
            ["ura", "loop", "status"].as_slice(),
            ["ura", "loop", "status", "--to", "kamo"].as_slice(),
            ["ura", "pair"].as_slice(),
            ["ura", "pair", "192.168.1.23"].as_slice(),
            [
                "ura",
                "pair",
                "192.168.1.23",
                "--code",
                "482913",
                "--device-name",
                "desuwa",
                "--name",
                "kamo",
                "--select",
            ]
            .as_slice(),
            ["ura", "device", "list"].as_slice(),
            [
                "ura",
                "device",
                "add",
                "kamo",
                "192.168.1.23",
                "--token",
                "secret",
            ]
            .as_slice(),
            ["ura", "device", "select"].as_slice(),
            ["ura", "device", "select", "kamo"].as_slice(),
            ["ura", "device", "remove", "kamo"].as_slice(),
            ["ura", "device", "authorize", "desuwa"].as_slice(),
            ["ura", "device", "revoke", "desuwa"].as_slice(),
        ] {
            Cli::try_parse_from(args).expect("current command should parse");
        }
    }

    #[test]
    fn legacy_serve_and_receiver_url_aliases_still_parse() {
        Cli::try_parse_from(["ura", "serve"]).expect("legacy serve alias should parse");
        Cli::try_parse_from([
            "ura",
            "--receiver-url",
            "http://127.0.0.1:8765",
            "status",
        ])
        .expect("legacy receiver-url alias should parse");
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
        for expected in ["daemon", "queue", "pause", "resume"] {
            assert!(
                subcommands.contains(&expected.to_string()),
                "help should contain {expected}"
            );
        }
        for legacy_or_removed in ["serve", "receive", "enqueue"] {
            assert!(
                !subcommands.contains(&legacy_or_removed.to_string()),
                "help should not advertise {legacy_or_removed}"
            );
        }

        let mut command = Cli::command();
        let mut help = Vec::new();
        command.write_long_help(&mut help).expect("write help");
        let help = String::from_utf8(help).expect("help should be UTF-8");
        assert!(help.contains("Add a media URL to the selected device's playback queue"));
        assert!(help.contains("Play audio on this or another paired Linux device"));
    }

    #[test]
    fn device_select_accepts_interactive_and_named_forms() {
        Cli::try_parse_from(["ura", "device", "select"])
            .expect("interactive device select should parse");
        Cli::try_parse_from(["ura", "device", "select", "living-room"])
            .expect("named device select should parse");
    }

    #[test]
    fn play_to_remains_a_one_shot_override() {
        Cli::try_parse_from([
            "ura",
            "play",
            "--to",
            "living-room",
            "https://youtu.be/example",
        ])
        .expect("play --to should parse");
    }
}
