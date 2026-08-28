use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use nicer::clipboard::{Clipboard, MemoryClipboard};
use nicer::config::{Config, Mode};
use nicer::error::Result;
use nicer::event::{Command, ConnectionId, Direction, Event, Verdict};
use nicer::identity::Fingerprint;
use nicer::node::{Node, NodeHandle, Reply};
use nicer::paths::Paths;
use nicer_proto::payload::OfferKind;
use tokio::sync::broadcast;

struct Shared(Arc<MemoryClipboard>);

impl Clipboard for Shared {
    fn read_text(&self) -> Result<String> {
        self.0.read_text()
    }

    fn write_text(&self, text: String) -> Result<()> {
        self.0.write_text(text)
    }
}

struct Peer {
    _dir: tempfile::TempDir,
    handle: NodeHandle,
    events: broadcast::Receiver<Event>,
    address: SocketAddr,
    fingerprint: Fingerprint,
    downloads: PathBuf,
    clipboard: Arc<MemoryClipboard>,
}

async fn start(name: &str, mode: Mode) -> Peer {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::rooted_at(dir.path());
    let downloads = dir.path().join("downloads");
    let clipboard = Arc::new(MemoryClipboard::default());

    let mut config = Config {
        device_name: name.to_string(),
        listen: "127.0.0.1:0".parse().unwrap(),
        discovery: false,
        download_dir: downloads.clone(),
        ..Config::default()
    };
    config.transport.mode = mode;
    config.transport.i_know_wireshark_can_read_this = mode == Mode::Plaintext;

    let (node, handle, inbox) =
        Node::build(config, &paths, Box::new(Shared(clipboard.clone()))).unwrap();
    let mut events = handle.subscribe();
    tokio::spawn(async move { node.run(inbox).await });

    let (address, fingerprint) = wait(&mut events, |event| match event {
        Event::Listening {
            address,
            fingerprint,
            ..
        } => Some((*address, *fingerprint)),
        _ => None,
    })
    .await;

    Peer {
        _dir: dir,
        handle,
        events,
        address,
        fingerprint,
        downloads,
        clipboard,
    }
}

async fn wait<T>(
    events: &mut broadcast::Receiver<Event>,
    matches: impl Fn(&Event) -> Option<T>,
) -> T {
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let event = events.recv().await.expect("the event stream ended");
            if let Some(found) = matches(&event) {
                return found;
            }
        }
    })
    .await
    .expect("timed out waiting for an event")
}

fn connection_of(event: &Event) -> Option<ConnectionId> {
    match event {
        Event::Connected { connection, .. } => Some(*connection),
        _ => None,
    }
}

async fn link(from: &mut Peer, to: &mut Peer) -> (ConnectionId, ConnectionId) {
    let reply = from
        .handle
        .call(Command::Connect {
            address: to.address,
        })
        .await;
    let Reply::Connected { connection } = reply else {
        panic!("connect failed: {reply:?}");
    };
    let inbound = wait(&mut to.events, connection_of).await;
    let _ = wait(&mut from.events, connection_of).await;
    (connection, inbound)
}

#[tokio::test]
async fn a_file_crosses_between_two_daemons_and_verifies_clean() {
    let mut sender = start("sender", Mode::Secure).await;
    let mut receiver = start("receiver", Mode::Secure).await;
    let (outbound, inbound) = link(&mut sender, &mut receiver).await;

    let body: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
    let source = sender._dir.path().join("shitpost.png");
    tokio::fs::write(&source, &body).await.unwrap();

    assert!(matches!(
        sender
            .handle
            .call(Command::SendFile {
                connection: outbound,
                path: source,
            })
            .await,
        Reply::Ok
    ));

    let (stream, size, name) = wait(&mut receiver.events, |event| match event {
        Event::Offer {
            stream, size, name, ..
        } => Some((*stream, *size, name.clone())),
        _ => None,
    })
    .await;
    assert_eq!(size, body.len() as u64);
    assert_eq!(name.as_deref(), Some("shitpost.png"));

    receiver
        .handle
        .call(Command::Respond {
            connection: inbound,
            stream,
            verdict: Verdict::Merge,
        })
        .await;

    let landed = wait(&mut receiver.events, |event| match event {
        Event::TransferComplete {
            direction: Direction::Incoming,
            path,
            ..
        } => Some(path.clone()),
        _ => None,
    })
    .await
    .expect("a received file must report where it landed");

    wait(&mut sender.events, |event| match event {
        Event::TransferComplete {
            direction: Direction::Outgoing,
            ..
        } => Some(()),
        Event::TransferFailed { reason, .. } => panic!("transfer failed: {reason}"),
        _ => None,
    })
    .await;

    assert_eq!(landed.parent().unwrap(), receiver.downloads);
    assert_eq!(tokio::fs::read(&landed).await.unwrap(), body);
}

#[tokio::test]
async fn a_refused_offer_writes_nothing() {
    let mut sender = start("sender", Mode::Secure).await;
    let mut receiver = start("receiver", Mode::Secure).await;
    let (outbound, inbound) = link(&mut sender, &mut receiver).await;

    let source = sender._dir.path().join("unwanted.bin");
    tokio::fs::write(&source, b"no thanks").await.unwrap();
    sender
        .handle
        .call(Command::SendFile {
            connection: outbound,
            path: source,
        })
        .await;

    let stream = wait(&mut receiver.events, |event| match event {
        Event::Offer { stream, .. } => Some(*stream),
        _ => None,
    })
    .await;

    receiver
        .handle
        .call(Command::Respond {
            connection: inbound,
            stream,
            verdict: Verdict::FuckOff {
                reason: Some("this meme sucks".into()),
            },
        })
        .await;

    let reason = wait(&mut sender.events, |event| match event {
        Event::OfferResolved {
            direction: Direction::Incoming,
            verdict: Verdict::FuckOff { reason },
            ..
        } => Some(reason.clone()),
        _ => None,
    })
    .await;
    assert_eq!(reason.as_deref(), Some("this meme sucks"));

    let written = std::fs::read_dir(&receiver.downloads)
        .map(|entries| entries.count())
        .unwrap_or(0);
    assert_eq!(written, 0, "a refused offer must leave the disk untouched");
}

#[tokio::test]
async fn clipboard_content_arrives_in_the_receivers_clipboard() {
    let mut sender = start("sender", Mode::Secure).await;
    let mut receiver = start("receiver", Mode::Secure).await;
    let (outbound, inbound) = link(&mut sender, &mut receiver).await;

    sender
        .handle
        .call(Command::SendClipboard {
            connection: outbound,
            text: Some("look at this...".into()),
        })
        .await;

    let (stream, kind, preview) = wait(&mut receiver.events, |event| match event {
        Event::Offer {
            stream,
            kind,
            preview,
            ..
        } => Some((*stream, *kind, preview.clone())),
        _ => None,
    })
    .await;
    assert_eq!(kind, OfferKind::Clipboard);
    assert_eq!(preview.as_deref(), Some("look at this..."));

    receiver
        .handle
        .call(Command::Respond {
            connection: inbound,
            stream,
            verdict: Verdict::Merge,
        })
        .await;

    wait(&mut receiver.events, |event| match event {
        Event::TransferComplete {
            direction: Direction::Incoming,
            ..
        } => Some(()),
        _ => None,
    })
    .await;

    assert_eq!(receiver.clipboard.read_text().unwrap(), "look at this...");
}

/// RFC R5: an identity is stored only when the user says so, and never by connecting.
#[tokio::test]
async fn connecting_announces_a_stranger_without_pairing_it() {
    let mut caller = start("caller", Mode::Secure).await;
    let mut callee = start("callee", Mode::Secure).await;
    let _ = link(&mut caller, &mut callee).await;

    let fingerprint = wait(&mut callee.events, |event| match event {
        Event::PairingRequired { fingerprint, .. } => Some(*fingerprint),
        _ => None,
    })
    .await;
    assert_eq!(fingerprint, caller.fingerprint);

    let Reply::Peers { peers } = callee.handle.call(Command::ListPeers).await else {
        panic!("ListPeers did not answer with peers");
    };
    assert!(peers.is_empty(), "a connection must not pair anything");

    callee
        .handle
        .call(Command::Pair {
            fingerprint,
            device: None,
        })
        .await;

    let Reply::Peers { peers } = callee.handle.call(Command::ListPeers).await else {
        panic!("ListPeers did not answer with peers");
    };
    assert_eq!(peers.len(), 1);
    assert_eq!(peers[0].fingerprint, caller.fingerprint);
    assert!(!peers[0].auto_accept_files);
}

#[tokio::test]
async fn an_auto_accepting_peer_needs_no_prompt() {
    let mut sender = start("sender", Mode::Secure).await;
    let mut receiver = start("receiver", Mode::Secure).await;

    receiver
        .handle
        .call(Command::Pair {
            fingerprint: sender.fingerprint,
            device: Some("sender".into()),
        })
        .await;
    receiver
        .handle
        .call(Command::AutoAccept {
            fingerprint: sender.fingerprint,
            kind: OfferKind::File,
            enabled: true,
        })
        .await;

    let (outbound, _) = link(&mut sender, &mut receiver).await;
    let source = sender._dir.path().join("welcome.txt");
    tokio::fs::write(&source, b"merged").await.unwrap();
    sender
        .handle
        .call(Command::SendFile {
            connection: outbound,
            path: source,
        })
        .await;

    let auto = wait(&mut receiver.events, |event| match event {
        Event::Offer { auto_accepted, .. } => Some(*auto_accepted),
        _ => None,
    })
    .await;
    assert!(auto, "a peer marked auto-accepting should not prompt");

    let landed = wait(&mut receiver.events, |event| match event {
        Event::TransferComplete {
            direction: Direction::Incoming,
            path,
            ..
        } => Some(path.clone()),
        _ => None,
    })
    .await
    .unwrap();
    assert_eq!(tokio::fs::read(&landed).await.unwrap(), b"merged");
}

/// Pairing during an open connection must take effect on that connection, not the next.
#[tokio::test]
async fn pairing_after_connecting_still_enables_auto_accept() {
    let mut sender = start("sender", Mode::Secure).await;
    let mut receiver = start("receiver", Mode::Secure).await;
    let (outbound, _) = link(&mut sender, &mut receiver).await;

    receiver
        .handle
        .call(Command::Pair {
            fingerprint: sender.fingerprint,
            device: None,
        })
        .await;
    receiver
        .handle
        .call(Command::AutoAccept {
            fingerprint: sender.fingerprint,
            kind: OfferKind::Clipboard,
            enabled: true,
        })
        .await;

    sender
        .handle
        .call(Command::SendClipboard {
            connection: outbound,
            text: Some("merged".into()),
        })
        .await;

    let auto = wait(&mut receiver.events, |event| match event {
        Event::Offer { auto_accepted, .. } => Some(*auto_accepted),
        _ => None,
    })
    .await;
    assert!(auto, "pairing mid-connection must be honoured immediately");

    wait(&mut receiver.events, |event| match event {
        Event::TransferComplete {
            direction: Direction::Incoming,
            ..
        } => Some(()),
        _ => None,
    })
    .await;
    assert_eq!(receiver.clipboard.read_text().unwrap(), "merged");
}

#[tokio::test]
async fn chat_reaches_the_other_side_labelled_by_address() {
    let mut a = start("a", Mode::Secure).await;
    let mut b = start("b", Mode::Secure).await;
    let (outbound, _) = link(&mut a, &mut b).await;

    a.handle
        .call(Command::Chat {
            connection: outbound,
            text: "this meme sucks".into(),
        })
        .await;

    let (text, from, stream) = wait(&mut b.events, |event| match event {
        Event::Chat {
            text, from, stream, ..
        } => Some((text.clone(), *from, *stream)),
        _ => None,
    })
    .await;

    assert_eq!(text, "this meme sucks");
    assert!(from.is_loopback());
    assert_eq!(stream % 2, 1, "the initiator's streams are odd");
}
