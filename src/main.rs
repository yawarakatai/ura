use anyhow::Result;
use clap::Parser;
use ura::{
    cli::{Cli, Command},
    config::Config,
};

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Receive => {
            println!("receive: placeholder; mpv startup and HTTP API are not implemented yet");
            Ok(())
        }
        Command::Play { url } => {
            let config = Config::load_with_overrides(cli.receiver_url, cli.token)?;
            println!(
                "play: placeholder; would send {url} to {} with configured token",
                config.receiver_url
            );
            Ok(())
        }
        Command::Enqueue { url } => {
            let config = Config::load_with_overrides(cli.receiver_url, cli.token)?;
            println!(
                "enqueue: placeholder; would send {url} to {} with configured token",
                config.receiver_url
            );
            Ok(())
        }
        Command::Toggle => {
            let config = Config::load_with_overrides(cli.receiver_url, cli.token)?;
            println!(
                "toggle: placeholder; would send toggle to {} with configured token",
                config.receiver_url
            );
            Ok(())
        }
        Command::Stop => {
            let config = Config::load_with_overrides(cli.receiver_url, cli.token)?;
            println!(
                "stop: placeholder; would send stop to {} with configured token",
                config.receiver_url
            );
            Ok(())
        }
        Command::Status => {
            let config = Config::load_with_overrides(cli.receiver_url, cli.token)?;
            println!(
                "status: placeholder; would request status from {} with configured token",
                config.receiver_url
            );
            Ok(())
        }
        Command::History => {
            let config = Config::load_with_overrides(cli.receiver_url, cli.token)?;
            println!(
                "history: placeholder; would request history from {} with configured token",
                config.receiver_url
            );
            Ok(())
        }
    }
}
