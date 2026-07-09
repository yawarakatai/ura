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
    /// Run the local audio receiver.
    Receive {
        /// Address for the HTTP receiver API to bind.
        #[arg(long)]
        bind: Option<SocketAddr>,
    },
    /// Play a URL immediately.
    Play { url: String },
    /// Add a URL to the playback queue.
    Enqueue { url: String },
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

        /// Bind address to write into config.toml for ura receive.
        #[arg(long, default_value = "127.0.0.1:8765")]
        bind: SocketAddr,
    },
}

#[derive(Debug, Subcommand)]
pub enum TokenCommand {
    /// Print a new random receiver token.
    Generate,
}
