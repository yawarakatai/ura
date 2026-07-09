use anyhow::Result;
use clap::Parser;
use ura::{
    cli::{Cli, Command, LoopCommand},
    config::Config,
    receiver::run_receive,
};

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Receive => run_receive(),
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
        Command::Loop { command } => {
            let config = Config::load_with_overrides(cli.receiver_url, cli.token)?;
            match command {
                LoopCommand::Off => println!(
                    "loop off: placeholder; would send loop-off to {} with configured token",
                    config.receiver_url
                ),
                LoopCommand::One => println!(
                    "loop one: placeholder; would send loop-one to {} with configured token",
                    config.receiver_url
                ),
                LoopCommand::Queue => println!(
                    "loop queue: placeholder; would send loop-queue to {} with configured token",
                    config.receiver_url
                ),
                LoopCommand::Status => println!(
                    "loop status: placeholder; would request loop status from {} with configured token",
                    config.receiver_url
                ),
            }
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
