use std::{net::SocketAddr, path::PathBuf};

use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "ura", version, about = "Play audio on this or another paired Linux device.")]
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
    /// Start the long-running local ura node.
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
    /// Manage receiver tokens.
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

        /// Receiver URL to write into config.toml for legacy clients.
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

#[derive(Debug, Subcommand)]
pub enum DeviceCommand {
    /// List this device, configured peers, and authorized controllers.
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
    /// Authorize a controller on this node.
    Authorize { name: String },
    /// Revoke a controller credential on this node.
    Revoke { name: String },
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

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
