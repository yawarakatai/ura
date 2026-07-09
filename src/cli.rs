use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "ura", version, about = "Tiny LAN/Tailscale audio caster")]
pub struct Cli {
    #[arg(long, global = true, env = "URA_RECEIVER_URL")]
    pub receiver_url: Option<String>,

    #[arg(long, global = true, env = "URA_TOKEN")]
    pub token: Option<String>,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Run the local audio receiver.
    Receive,
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
        command: LoopCommand,
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
    One,
    /// Loop the playback queue forever.
    Queue,
    /// Show the current loop mode.
    Status,
}
