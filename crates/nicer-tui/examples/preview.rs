//! Renders the UI against synthetic state and dumps it as text, for eyeballing layout
//! changes without a real terminal: `cargo run -p nicer-tui --example preview`.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;

use nicer::clipboard;
use nicer::config::Config;
use nicer::event::{ConnectionId, ConnectionSummary, Direction};
use nicer::identity::Fingerprint;
use nicer::node::Node;
use nicer::pairing::PairedPeer;
use nicer::paths::Paths;
use nicer_proto::payload::{OfferKind, TransportMode};
use nicer_tui::app::{App, ChatLine, Discovered, RequestKind, Tab, TransferEntry, TransferStatus};
use nicer_tui::ui;
use ratatui::backend::TestBackend;
use ratatui::Terminal;
use tokio::sync::mpsc;

fn fp(seed: u8) -> Fingerprint {
    Fingerprint::of_spki(&[seed; 32])
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let dir = tempdir();
    let paths = Paths::rooted_at(&dir);
    let mut config = Config::load(&paths)?;
    config.listen = "127.0.0.1:0".parse().unwrap();
    config.discovery = false;
    let (node, handle, _inbox) = Node::build(config, &paths, clipboard::system_or_memory())?;
    drop(node);

    let (tx, _rx) = mpsc::unbounded_channel();
    let mut app = App::new(handle, tx);
    app.device = "alpha".into();
    app.fingerprint = Some(fp(1));
    app.listen = Some("0.0.0.0:6969".parse().unwrap());
    app.dispatch(RequestKind::Status, nicer::event::Command::Status);

    app.peers.push(PairedPeer {
        fingerprint: fp(2),
        device: "thinkpad".into(),
        paired_at: 0,
        last_seen: Some(0),
        last_address: Some(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 42))),
        auto_accept_files: false,
        auto_accept_clipboard: true,
    });

    app.discovered.insert(
        "framework".into(),
        Discovered {
            device: "framework".into(),
            addresses: vec![IpAddr::V4(Ipv4Addr::new(192, 168, 1, 77))],
            port: 6969,
            secure: true,
            fingerprint: Some(fp(3)),
        },
    );

    let conn = ConnectionId(1);
    app.connections.insert(
        conn,
        ConnectionSummary {
            connection: conn,
            address: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 42)), 6969),
            device: "thinkpad".into(),
            transport: TransportMode::Secure,
            direction: Direction::Outgoing,
            fingerprint: Some(fp(2)),
            paired: true,
        },
    );

    app.transfers.push(TransferEntry {
        connection: conn,
        stream: 8,
        name: Some("extremely_important_cat.png".into()),
        status: TransferStatus::Active {
            direction: Direction::Incoming,
            kind: OfferKind::File,
            transferred: 400_000,
            total: 812_944,
        },
        seq: 1,
    });
    app.transfers.push(TransferEntry {
        connection: conn,
        stream: 9,
        name: Some("meeting notes".into()),
        status: TransferStatus::Complete {
            direction: Direction::Outgoing,
            kind: OfferKind::Clipboard,
            path: None,
        },
        seq: 2,
    });

    app.chats.entry(conn).or_default().extend([
        ChatLine {
            from_us: false,
            from: "192.168.1.42".into(),
            fingerprint: Some(fp(2)),
            text: "this meme sucks".into(),
        },
        ChatLine {
            from_us: true,
            from: "you".into(),
            fingerprint: Some(fp(1)),
            text: "merged".into(),
        },
    ]);
    app.chat_connection = Some(conn);

    for (opcode, detail, color) in [
        ("DIFF", "400000/812944", nicer_tui::theme::SIGNAL),
        ("MERGE", "accepted", nicer_tui::theme::NICE),
        ("PULL_REQUEST", "file cat.png (812944B)", nicer_tui::theme::SIGNAL),
        ("MERGED", "we accepted", nicer_tui::theme::NICE),
    ] {
        app.ticker.push_front(nicer_tui::wire::Tick {
            opcode: opcode.into(),
            detail: detail.into(),
            color,
        });
    }

    app.set_status("pair ok", nicer_tui::theme::NICE);

    let backend = TestBackend::new(120, 40);
    let mut terminal = Terminal::new(backend)?;

    for tab in Tab::ALL {
        app.tab = tab;
        terminal.draw(|f| ui::draw(f, &app))?;
        println!("=== {} ===", tab.title());
        println!("{}", render(terminal.backend()));
    }

    Ok(())
}

fn render(backend: &TestBackend) -> String {
    let buffer = backend.buffer();
    let area = buffer.area;
    let mut out = String::new();
    for y in 0..area.height {
        for x in 0..area.width {
            out.push_str(buffer[(x, y)].symbol());
        }
        out.push('\n');
    }
    out
}

fn tempdir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("nicer-tui-preview-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}
