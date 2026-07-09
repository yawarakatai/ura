use anyhow::Result;
use clap::Parser;
use ura::{
    cli::{Cli, Command, LoopCommand},
    client::HttpClient,
    config::Config,
    receiver::run_receive,
};

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Receive { bind } => {
            let token = Config::load_token_with_override(cli.token)?;
            run_receive(bind, token).await
        }
        Command::Play { url } => {
            let config = Config::load_with_overrides(cli.receiver_url, cli.token)?;
            let client = HttpClient::new(config.receiver_url, config.token)?;
            client.play(&url)?;
            println!("play: sent {url}");
            Ok(())
        }
        Command::Enqueue { url } => {
            let config = Config::load_with_overrides(cli.receiver_url, cli.token)?;
            let client = HttpClient::new(config.receiver_url, config.token)?;
            client.enqueue(&url)?;
            println!("enqueue: sent {url}");
            Ok(())
        }
        Command::Toggle => {
            let config = Config::load_with_overrides(cli.receiver_url, cli.token)?;
            let client = HttpClient::new(config.receiver_url, config.token)?;
            client.control("toggle")?;
            println!("toggle: sent");
            Ok(())
        }
        Command::Stop => {
            let config = Config::load_with_overrides(cli.receiver_url, cli.token)?;
            let client = HttpClient::new(config.receiver_url, config.token)?;
            client.control("stop")?;
            println!("stop: sent");
            Ok(())
        }
        Command::Loop { command } => {
            let config = Config::load_with_overrides(cli.receiver_url, cli.token)?;
            let client = HttpClient::new(config.receiver_url, config.token)?;
            match command {
                LoopCommand::Off => {
                    client.control("loop-off")?;
                    println!("loop off: sent");
                }
                LoopCommand::One => {
                    client.control("loop-one")?;
                    println!("loop one: sent");
                }
                LoopCommand::Queue => {
                    client.control("loop-queue")?;
                    println!("loop queue: sent");
                }
                LoopCommand::Status => {
                    let status = client.loop_status()?;
                    println!("loop status: {status:?}");
                }
            }
            Ok(())
        }
        Command::Status => {
            let config = Config::load_with_overrides(cli.receiver_url, cli.token)?;
            let client = HttpClient::new(config.receiver_url, config.token)?;
            let status = client.status()?;
            println!("{}", serde_json::to_string_pretty(&status)?);
            Ok(())
        }
        Command::History => {
            let config = Config::load_with_overrides(cli.receiver_url, cli.token)?;
            let client = HttpClient::new(config.receiver_url, config.token)?;
            let history = client.history()?;
            println!("{}", serde_json::to_string_pretty(&history)?);
            Ok(())
        }
    }
}
