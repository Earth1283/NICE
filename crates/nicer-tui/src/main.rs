use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::Parser;
use crossterm::event::{Event as CtEvent, EventStream, KeyEventKind};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::ExecutableCommand;
use futures_util::StreamExt;
use nicer::clipboard;
use nicer::config::{Config, Mode, PLAINTEXT_WARNING};
use nicer::control;
use nicer::event::Command;
use nicer::node::Node;
use nicer::paths::Paths;
use nicer_tui::app::{App, RequestKind};
use nicer_tui::{theme, ui};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use tokio::sync::mpsc;

#[derive(Parser)]
#[command(name = "nicer-tui", version, about = "A ratatui frontend for the NICE/1 daemon")]
struct Cli {
    /// Directory holding config.toml, overriding the platform default.
    #[arg(long, value_name = "DIR")]
    config_dir: Option<PathBuf>,

    /// Keep every file this instance writes under one directory.
    #[arg(long, value_name = "DIR")]
    root: Option<PathBuf>,

    /// Address to listen on for NICE/1 peers.
    #[arg(long, value_name = "ADDR")]
    listen: Option<SocketAddr>,

    /// Name shown to peers and advertised over mDNS.
    #[arg(long, value_name = "NAME")]
    device: Option<String>,

    /// Do not advertise or browse over mDNS.
    #[arg(long)]
    no_discovery: bool,

    /// Ask for PLAINTEXT. This alone is not enough: the configuration file must also set
    /// transport.i_know_wireshark_can_read_this.
    #[arg(long)]
    plaintext: bool,

    /// Also serve the NDJSON control socket at this path, for other frontends.
    #[arg(long, value_name = "PATH")]
    socket: Option<PathBuf>,
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

fn load_config(paths: &Paths, cli: &Cli) -> Result<Config> {
    let mut config = Config::load(paths)?;
    if let Some(listen) = cli.listen {
        config.listen = listen;
    }
    if let Some(device) = &cli.device {
        config.device_name = device.clone();
    }
    if cli.no_discovery {
        config.discovery = false;
    }
    if cli.plaintext {
        config.transport.mode = Mode::Plaintext;
    }
    config
        .validate()
        .context("the daemon refused to start with this configuration")?;
    Ok(config)
}

struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = std::io::stdout().execute(LeaveAlternateScreen);
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let paths = resolve_paths(&cli)?;
    let config = load_config(&paths, &cli)?;

    if config.mode() == Mode::Plaintext {
        eprintln!("{PLAINTEXT_WARNING}");
    }

    let (node, handle, inbox) = Node::build(config, &paths, clipboard::system_or_memory())?;

    // Subscribe before the node starts running, so nothing between `Listening` and our
    // first `status` call is lost — the same ordering the control-socket docs require.
    let events = handle.subscribe();

    let node_task = tokio::spawn(node.run(inbox));

    let control_task = cli.socket.clone().map(|endpoint| {
        let handle = handle.clone();
        tokio::spawn(async move { control::serve(&endpoint, handle).await })
    });

    let (results_tx, mut results_rx) = mpsc::unbounded_channel();
    let mut app = App::new(handle.clone(), results_tx);
    app.dispatch(RequestKind::Status, Command::Status);
    app.dispatch(RequestKind::ListPeers, Command::ListPeers);
    app.dispatch(RequestKind::ListConnections, Command::ListConnections);

    enable_raw_mode()?;
    std::io::stdout().execute(EnterAlternateScreen)?;
    let _guard = TerminalGuard;
    let mut terminal = Terminal::new(CrosstermBackend::new(std::io::stdout()))?;
    terminal.clear()?;

    let mut keys = EventStream::new();
    let mut events = events;

    let result = run(&mut terminal, &mut app, &mut keys, &mut events, &mut results_rx).await;

    drop(app);
    if let Some(control_task) = control_task {
        control_task.abort();
    }
    node_task.abort();
    if cli.socket.is_some() {
        // best-effort; the daemon side also cleans this up on a normal exit
    }

    result
}

async fn run(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    app: &mut App,
    keys: &mut EventStream,
    events: &mut tokio::sync::broadcast::Receiver<nicer::event::Event>,
    results_rx: &mut mpsc::UnboundedReceiver<(RequestKind, nicer::node::Reply)>,
) -> Result<()> {
    terminal.draw(|f| ui::draw(f, app))?;

    loop {
        tokio::select! {
            key = keys.next() => {
                match key {
                    Some(Ok(CtEvent::Key(key))) if key.kind == KeyEventKind::Press => {
                        app.on_key(key);
                    }
                    Some(Ok(_)) => {}
                    Some(Err(e)) => return Err(e.into()),
                    None => break,
                }
            }
            event = events.recv() => {
                match event {
                    Ok(event) => app.on_event(event),
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => app.on_lagged(n),
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        app.set_status("the daemon stopped", theme::CUT);
                        app.should_quit = true;
                    }
                }
            }
            reply = results_rx.recv() => {
                if let Some((kind, reply)) = reply {
                    app.on_reply(kind, reply);
                }
            }
            _ = tokio::time::sleep(Duration::from_millis(250)) => {}
        }

        terminal.draw(|f| ui::draw(f, app))?;

        if app.should_quit {
            break;
        }
    }

    Ok(())
}
