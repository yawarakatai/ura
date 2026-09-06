use std::{net::SocketAddr, num::NonZeroUsize};

use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    name = "ura",
    version,
    about = "Play audio on this or another paired Linux device.",
    override_usage = "ura [OPTIONS] [URL]\n       ura <COMMAND>",
    args_conflicts_with_subcommands = true,
    disable_help_subcommand = true
)]
pub struct Cli {
    /// YouTube or YouTube Music URL to play.
    #[arg(value_name = "URL", value_parser = parse_url)]
    pub url: Option<String>,

    /// Add URL to the playback queue instead of playing it immediately.
    #[arg(short, long, requires = "url", conflicts_with = "loop_track")]
    pub queue: bool,

    /// Loop URL until another URL is played.
    #[arg(short = 'l', long = "loop", requires = "url", conflicts_with = "queue")]
    pub loop_track: bool,

    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Toggle pause/resume on the selected device.
    Toggle,
    /// Stop playback on the selected device.
    Stop,
    /// List playback history or replay an entry by number.
    History {
        /// Latest-first history number to replay. Omit to list history.
        #[arg(value_name = "NUMBER")]
        index: Option<NonZeroUsize>,

        /// Add the history entry to the playback queue.
        #[arg(short, long, requires = "index", conflicts_with = "loop_track")]
        queue: bool,

        /// Loop the history entry until another URL is played.
        #[arg(
            short = 'l',
            long = "loop",
            requires = "index",
            conflicts_with = "queue"
        )]
        loop_track: bool,
    },
    /// Choose and manage playback devices.
    #[command(disable_help_subcommand = true)]
    Device {
        #[command(subcommand)]
        command: DeviceCommand,
    },
    /// Open pairing on this node, or pair with another node by address.
    Pair {
        /// Peer address to pair with. Omit to open pairing on this node.
        address: Option<String>,
        /// Six-digit pairing code shown on the peer.
        #[arg(long)]
        code: Option<String>,
        /// This node's name stored on the peer.
        #[arg(long = "node-name", alias = "device-name")]
        node_name: Option<String>,
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
    /// Run the long-running local ura node.
    Daemon {
        /// Address for the peer HTTP API to bind.
        #[arg(long)]
        bind: Option<SocketAddr>,
    },
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

fn parse_url(value: &str) -> Result<String, String> {
    let value = value.trim();
    let scheme = value
        .split_once("://")
        .map(|(scheme, _)| scheme)
        .unwrap_or_default();
    if value.chars().any(char::is_whitespace)
        || !(scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https"))
    {
        return Err("expected an http:// or https:// media URL".to_string());
    }
    Ok(value.to_string())
}

#[cfg(test)]
mod tests {
    use clap::{CommandFactory, Parser};

    use super::*;

    #[test]
    fn parses_status_and_url_playback_forms() {
        Cli::try_parse_from(["ura"]).expect("bare command should show status");
        Cli::try_parse_from(["ura", "https://youtu.be/example"])
            .expect("URL should play immediately");
        Cli::try_parse_from(["ura", "--queue", "https://youtu.be/example"])
            .expect("queue option should parse");
        Cli::try_parse_from(["ura", "https://youtu.be/example", "--loop"])
            .expect("loop option should parse");
        Cli::try_parse_from(["ura", "-q", "https://youtu.be/example"])
            .expect("short queue option should parse");
        Cli::try_parse_from(["ura", "-l", "https://youtu.be/example"])
            .expect("short loop option should parse");
    }

    #[test]
    fn parses_structured_commands() {
        for args in [
            ["ura", "toggle"].as_slice(),
            ["ura", "stop"].as_slice(),
            ["ura", "history"].as_slice(),
            ["ura", "history", "1"].as_slice(),
            ["ura", "history", "3", "--queue"].as_slice(),
            ["ura", "history", "--loop", "3"].as_slice(),
            ["ura", "pair"].as_slice(),
            ["ura", "pair", "192.168.1.23"].as_slice(),
            [
                "ura",
                "pair",
                "192.168.1.23",
                "--code",
                "482913",
                "--node-name",
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
            ["ura", "daemon"].as_slice(),
            ["ura", "daemon", "--bind", "127.0.0.1:9876"].as_slice(),
        ] {
            Cli::try_parse_from(args).expect("public command should parse");
        }
    }

    #[test]
    fn rejects_ambiguous_or_incomplete_url_options() {
        Cli::try_parse_from(["ura", "--queue"]).expect_err("queue requires a URL");
        Cli::try_parse_from(["ura", "--loop"]).expect_err("loop requires a URL");
        Cli::try_parse_from(["ura", "--queue", "--loop", "https://youtu.be/example"])
            .expect_err("queue and loop should conflict");
        Cli::try_parse_from(["ura", "--loop", "toggle"])
            .expect_err("root options should conflict with subcommands");
        Cli::try_parse_from(["ura", "history", "--queue"])
            .expect_err("history queue requires an entry number");
        Cli::try_parse_from(["ura", "history", "--loop"])
            .expect_err("history loop requires an entry number");
        Cli::try_parse_from(["ura", "history", "1", "--queue", "--loop"])
            .expect_err("history queue and loop should conflict");
    }

    #[test]
    fn rejects_unknown_commands_as_non_urls() {
        let error = Cli::try_parse_from(["ura", "toggel"])
            .expect_err("a command typo should not be treated as a URL");

        assert!(
            error
                .to_string()
                .contains("expected an http:// or https://")
        );
    }

    #[test]
    fn removed_commands_and_options_are_rejected() {
        for args in [
            ["ura", "play", "https://youtu.be/example"].as_slice(),
            ["ura", "queue", "https://youtu.be/example"].as_slice(),
            ["ura", "pause"].as_slice(),
            ["ura", "resume"].as_slice(),
            ["ura", "status"].as_slice(),
            ["ura", "loop", "track"].as_slice(),
            ["ura", "playback", "status"].as_slice(),
            ["ura", "config", "init"].as_slice(),
            ["ura", "token", "generate"].as_slice(),
            ["ura", "--to", "kamo"].as_slice(),
            ["ura", "--config", "/tmp/config.toml"].as_slice(),
            ["ura", "--peer-url", "http://127.0.0.1:8765"].as_slice(),
            ["ura", "--token", "secret"].as_slice(),
            ["ura", "history", "list"].as_slice(),
            ["ura", "history", "replay"].as_slice(),
            ["ura", "help"].as_slice(),
        ] {
            Cli::try_parse_from(args).expect_err("removed interface should be rejected");
        }
    }

    #[test]
    fn help_shows_only_the_small_top_level_command_set() {
        let command = Cli::command();
        let subcommands = command
            .get_subcommands()
            .map(|command| command.get_name())
            .collect::<Vec<_>>();

        assert_eq!(
            subcommands,
            ["toggle", "stop", "history", "device", "pair", "daemon"]
        );

        let mut command = Cli::command();
        let mut help = Vec::new();
        command.write_long_help(&mut help).expect("write help");
        let help = String::from_utf8(help).expect("help should be UTF-8");
        assert!(help.contains("ura [OPTIONS] [URL]"));
        assert!(help.contains("--queue"));
        assert!(help.contains("--loop"));
        assert!(!help.contains("  help"));
        assert!(!help.contains("\n  playback"));
    }

    #[test]
    fn nested_help_subcommands_are_disabled() {
        let command = Cli::command();
        let device = command.find_subcommand("device").expect("device command");
        assert!(
            device.find_subcommand("help").is_none(),
            "device should use --help instead of a help subcommand"
        );
    }

    #[test]
    fn history_replay_uses_one_based_indices() {
        Cli::try_parse_from(["ura", "history", "1"]).expect("positive history index should parse");
        Cli::try_parse_from(["ura", "history", "0"])
            .expect_err("zero history index should be rejected");
    }
}
