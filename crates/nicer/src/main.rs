use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use nicer::clipboard;
use nicer::config::{Config, Mode, PLAINTEXT_WARNING};
use nicer::control;
use nicer::identity::Identity;
use nicer::node::Node;
use nicer::paths::Paths;

#[derive(Parser)]
#[command(name = "nicer", version, about = "The NICE/1 daemon")]
struct Cli {
    /// Directory holding config.toml, overriding the platform default.
    #[arg(long, value_name = "DIR", global = true)]
    config_dir: Option<PathBuf>,

    /// Keep every file this daemon writes under one directory. Useful for running two
    /// nodes on one machine.
    #[arg(long, value_name = "DIR", global = true)]
    root: Option<PathBuf>,

    #[command(subcommand)]
    command: Option<Sub>,
}

#[derive(Subcommand)]
enum Sub {
    /// Run the daemon. This is the default.
    Run(RunArgs),
    /// Print this device's identity and exit.
    Identity,
    /// Print the effective configuration and exit.
    Config,
}

#[derive(clap::Args, Default)]
struct RunArgs {
    /// Address to listen on for NICE/1 peers.
    #[arg(long, value_name = "ADDR")]
    listen: Option<SocketAddr>,

    /// Name shown to peers and advertised over mDNS.
    #[arg(long, value_name = "NAME")]
    device: Option<String>,

    /// Path of the control socket, or named pipe on Windows.
    #[arg(long, value_name = "PATH")]
    socket: Option<PathBuf>,

    /// Do not advertise or browse over mDNS.
    #[arg(long)]
    no_discovery: bool,

    /// Ask for PLAINTEXT. This alone is not enough: the configuration file must also set
    /// transport.i_know_wireshark_can_read_this.
    #[arg(long)]
    plaintext: bool,
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("NICER_LOG")
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();
    let paths = resolve_paths(&cli)?;

    match cli.command {
        Some(Sub::Identity) => show_identity(&paths),
        Some(Sub::Config) => show_config(&paths),
        Some(Sub::Run(args)) => run(paths, args),
        None => run(paths, RunArgs::default()),
    }
}

fn resolve_paths(cli: &Cli) -> Result<Paths> {
    let mut paths = match &cli.root {
        Some(root) => Paths::rooted_at(root),
        None => Paths::discover()?,
    };
    if let Some(config_dir) = &cli.config_dir {
        paths.config_dir = config_dir.clone();
    }
    Ok(paths)
}

fn load(paths: &Paths, args: &RunArgs) -> Result<Config> {
    let mut config = Config::load(paths)?;
    if let Some(listen) = args.listen {
        config.listen = listen;
    }
    if let Some(device) = &args.device {
        config.device_name = device.clone();
    }
    if args.no_discovery {
        config.discovery = false;
    }
    if args.plaintext {
        config.transport.mode = Mode::Plaintext;
    }
    config
        .validate()
        .context("the daemon refused to start with this configuration")?;
    Ok(config)
}

fn show_identity(paths: &Paths) -> Result<()> {
    let config = Config::load(paths)?;
    paths.create_all()?;
    let identity = Identity::load_or_generate(&paths.identity_file(), &config.device_name)?;
    println!("device      {}", identity.device_name());
    println!("fingerprint {}", identity.fingerprint());
    println!("short       {}", identity.fingerprint().short());
    Ok(())
}

fn show_config(paths: &Paths) -> Result<()> {
    let config = Config::load(paths)?;
    println!("# {}", paths.config_file().display());
    print!("{}", toml::to_string_pretty(&config)?);
    Ok(())
}

fn run(paths: Paths, args: RunArgs) -> Result<()> {
    let config = load(&paths, &args)?;
    let endpoint = args
        .socket
        .clone()
        .unwrap_or_else(|| paths.control_socket());

    if config.mode() == Mode::Plaintext {
        eprintln!("{PLAINTEXT_WARNING}");
    }
    if config.keylog.is_some() {
        eprintln!("WARNING: TLS session keys are being written for Wireshark");
    }

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    runtime.block_on(async move {
        let (node, handle, inbox) = Node::build(config, &paths, clipboard::system_or_memory())?;
        let control = tokio::spawn({
            let handle = handle.clone();
            let endpoint = endpoint.clone();
            async move { control::serve(&endpoint, handle).await }
        });

        let outcome = tokio::select! {
            result = node.run(inbox) => result.map_err(anyhow::Error::from),
            result = control => match result {
                Ok(result) => result.map_err(anyhow::Error::from),
                Err(e) => Err(anyhow::Error::from(e)),
            },
            _ = tokio::signal::ctrl_c() => {
                tracing::info!("shutting down");
                Ok(())
            }
        };

        if cfg!(unix) {
            let _ = std::fs::remove_file(&endpoint);
        }
        outcome
    })
}
