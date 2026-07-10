use anyhow::Result;
use clap::Parser;
use ura::{
    cli::{Cli, Command, ConfigCommand, LoopCommand, TokenCommand},
    client::HttpClient,
    config::{Config, ConfigInit, ReceiverConfig, generate_token},
    mpv::LoopStatus,
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
        Command::Play { url } => {
            let config = Config::load_with_overrides(cli.config, cli.receiver_url, cli.token)?;
            let client = HttpClient::new(config.receiver_url, config.token)?;
            client.play(&url)?;
            println!("play: sent {url}");
            Ok(())
        }
        Command::Queue { url } => {
            let config = Config::load_with_overrides(cli.config, cli.receiver_url, cli.token)?;
            let client = HttpClient::new(config.receiver_url, config.token)?;
            client.queue(&url)?;
            println!("queue: sent {url}");
            Ok(())
        }
        Command::Pause => {
            let config = Config::load_with_overrides(cli.config, cli.receiver_url, cli.token)?;
            let client = HttpClient::new(config.receiver_url, config.token)?;
            client.control("pause")?;
            println!("pause: sent");
            Ok(())
        }
        Command::Resume => {
            let config = Config::load_with_overrides(cli.config, cli.receiver_url, cli.token)?;
            let client = HttpClient::new(config.receiver_url, config.token)?;
            client.control("resume")?;
            println!("resume: sent");
            Ok(())
        }
        Command::Toggle => {
            let config = Config::load_with_overrides(cli.config, cli.receiver_url, cli.token)?;
            let client = HttpClient::new(config.receiver_url, config.token)?;
            client.control("toggle")?;
            println!("toggle: sent");
            Ok(())
        }
        Command::Stop => {
            let config = Config::load_with_overrides(cli.config, cli.receiver_url, cli.token)?;
            let client = HttpClient::new(config.receiver_url, config.token)?;
            client.control("stop")?;
            println!("stop: sent");
            Ok(())
        }
        Command::Loop { command } => {
            let config = Config::load_with_overrides(cli.config, cli.receiver_url, cli.token)?;
            let client = HttpClient::new(config.receiver_url, config.token)?;
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
                Some(LoopCommand::Off) => {
                    client.control("loop-off")?;
                    println!("loop off: sent");
                }
                Some(LoopCommand::Track) => {
                    client.control("loop-one")?;
                    println!("loop track: sent");
                }
                Some(LoopCommand::Queue) => {
                    client.control("loop-queue")?;
                    println!("loop queue: sent");
                }
                Some(LoopCommand::Status) => {
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
        Command::Status => {
            let config = Config::load_with_overrides(cli.config, cli.receiver_url, cli.token)?;
            let client = HttpClient::new(config.receiver_url, config.token)?;
            let status = client.status()?;
            println!("{}", serde_json::to_string_pretty(&status)?);
            Ok(())
        }
        Command::History => {
            let config = Config::load_with_overrides(cli.config, cli.receiver_url, cli.token)?;
            let client = HttpClient::new(config.receiver_url, config.token)?;
            let history = client.history()?;
            println!("{}", serde_json::to_string_pretty(&history)?);
            Ok(())
        }
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
