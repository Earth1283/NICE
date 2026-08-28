use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use nicer::clipboard::MemoryClipboard;
use nicer::config::{Config, Mode};
use nicer::event::{Command, ConnectionId, Event, Verdict};
use nicer::node::{Node, NodeHandle};
use nicer::paths::Paths;
use nicer::transfer::{sanitize_filename, unique_path};
use nicer_proto::payload::{Hello, Lkml, OfferKind, PullRequest, TransportMode};
use nicer_proto::{Diff, Frame, Incoming, NiceCodec, Opcode, StreamId, MAGIC};
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio::sync::broadcast;
use tokio_util::codec::Framed;

struct Host {
    _dir: tempfile::TempDir,
    handle: NodeHandle,
    events: broadcast::Receiver<Event>,
    address: SocketAddr,
    downloads: PathBuf,
}

/// A node speaking PLAINTEXT, so a hand-rolled peer can misbehave at it directly.
async fn host() -> Host {
    host_with(|_| {}).await
}

async fn host_with(adjust: impl FnOnce(&mut Config)) -> Host {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::rooted_at(dir.path());
    let downloads = dir.path().join("downloads");

    let mut config = Config {
        device_name: "host".into(),
        listen: "127.0.0.1:0".parse().unwrap(),
        discovery: false,
        download_dir: downloads.clone(),
        ..Config::default()
    };
    config.transport.mode = Mode::Plaintext;
    config.transport.i_know_wireshark_can_read_this = true;
    adjust(&mut config);

    let (node, handle, inbox) =
        Node::build(config, &paths, Box::new(MemoryClipboard::default())).unwrap();
    let mut events = handle.subscribe();
    tokio::spawn(async move { node.run(inbox).await });

    let address = wait(&mut events, |event| match event {
        Event::Listening { address, .. } => Some(*address),
        _ => None,
    })
    .await;

    Host {
        _dir: dir,
        handle,
        events,
        address,
        downloads,
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

type Wire = Framed<TcpStream, NiceCodec>;

async fn greet(address: SocketAddr) -> Wire {
    let stream = TcpStream::connect(address).await.unwrap();
    let mut wire = Framed::new(stream, NiceCodec::new(1024 * 1024));
    let hello = Hello {
        version: 1,
        device: "hand-rolled".into(),
        fingerprint: "00".repeat(32),
        transport: TransportMode::Plaintext,
        max_frame_size: 1024 * 1024,
        capabilities: Vec::new(),
    };
    wire.send(Frame::control_cbor(Opcode::Hello, &hello).unwrap())
        .await
        .unwrap();
    assert_eq!(read(&mut wire).await.unwrap().opcode, Opcode::Merged);
    wire
}

/// `None` means the host closed the connection.
async fn read(wire: &mut Wire) -> Option<Frame> {
    match tokio::time::timeout(Duration::from_secs(10), wire.next()).await {
        Ok(Some(Ok(Incoming::Frame(frame)))) => Some(frame),
        Ok(Some(Ok(Incoming::Unknown { opcode, .. }))) => {
            panic!("host sent undefined opcode 0x{opcode:02x}")
        }
        Ok(Some(Err(e))) => panic!("host sent an undecodable frame: {e}"),
        Ok(None) => None,
        Err(_) => panic!("the host said nothing"),
    }
}

async fn expect_fatal(wire: &mut Wire, opcode: Opcode) {
    let frame = read(wire).await.expect("the host closed without answering");
    assert_eq!(frame.opcode, opcode, "expected {opcode}");
    assert!(
        read(wire).await.is_none(),
        "{opcode} must be followed by a close"
    );
}

/// `NICE-1.md` §20 names this exact sequence as the canonical protocol violation.
#[tokio::test]
async fn diff_before_merge_is_broke_userspace() {
    let host = host().await;
    let mut wire = greet(host.address).await;

    let offer = PullRequest {
        kind: OfferKind::File,
        size: 4,
        mime: None,
        name: Some("premature.bin".into()),
        preview: None,
    };
    wire.send(Frame::cbor(Opcode::PullRequest, StreamId(1), &offer).unwrap())
        .await
        .unwrap();
    wire.send(
        Diff {
            offset: 0,
            data: Bytes::from_static(b"oops"),
        }
        .into_frame(StreamId(1)),
    )
    .await
    .unwrap();

    expect_fatal(&mut wire, Opcode::BrokeUserspace).await;
}

/// RFC R3: an initiator that opens an even stream has broken the partition.
#[tokio::test]
async fn a_stream_of_the_wrong_parity_is_broke_userspace() {
    let host = host().await;
    let mut wire = greet(host.address).await;

    let offer = PullRequest {
        kind: OfferKind::Clipboard,
        size: 1,
        mime: None,
        name: None,
        preview: None,
    };
    wire.send(Frame::cbor(Opcode::PullRequest, StreamId(2), &offer).unwrap())
        .await
        .unwrap();

    expect_fatal(&mut wire, Opcode::BrokeUserspace).await;
}

/// RFC R3: `TUX` is connection-level and belongs on stream 0.
#[tokio::test]
async fn a_connection_level_opcode_on_a_stream_is_rejected() {
    let host = host().await;
    let mut wire = greet(host.address).await;

    wire.send(Frame::bare(Opcode::Tux, StreamId(1)))
        .await
        .unwrap();
    expect_fatal(&mut wire, Opcode::BrokeUserspace).await;
}

/// RFC R1: an undefined opcode earns `CPP` and leaves the connection usable, because a
/// future revision has to be deployable against this one.
#[tokio::test]
async fn an_undefined_opcode_earns_cpp_and_the_connection_survives() {
    let host = host().await;
    let mut wire = greet(host.address).await;

    let raw = [MAGIC, 0x01, 0x5A, 0x00, 0, 0, 0, 1, 0, 0, 0, 0];
    wire.get_mut().write_all(&raw).await.unwrap();

    let answer = read(&mut wire).await.expect("CPP was expected");
    assert_eq!(answer.opcode, Opcode::Cpp);

    wire.send(Frame::control(Opcode::Tux)).await.unwrap();
    assert_eq!(
        read(&mut wire)
            .await
            .expect("the connection should survive")
            .opcode,
        Opcode::Subsurface
    );
}

/// RFC R2: the length is refused from the header, so the payload is never allocated.
#[tokio::test]
async fn an_oversized_frame_ends_the_connection_without_allocating_it() {
    let host = host().await;
    let mut wire = greet(host.address).await;

    let mut raw = [
        MAGIC,
        0x01,
        Opcode::Diff as u8,
        0x00,
        0,
        0,
        0,
        1,
        0,
        0,
        0,
        0,
    ];
    raw[8..12].copy_from_slice(&(64u32 * 1024 * 1024).to_be_bytes());
    wire.get_mut().write_all(&raw).await.unwrap();

    assert!(
        read(&mut wire).await.is_none(),
        "framing is lost, so the host must close rather than reply"
    );
}

/// RFC R10: one conversation per connection.
#[tokio::test]
async fn a_second_conversation_stream_is_broke_userspace() {
    let host = host().await;
    let mut wire = greet(host.address).await;

    let message = |text: &str| Lkml {
        message_id: text.into(),
        timestamp: 0,
        text: text.into(),
        tags: None,
    };
    wire.send(Frame::cbor(Opcode::Lkml, StreamId(1), &message("merged")).unwrap())
        .await
        .unwrap();
    wire.send(Frame::cbor(Opcode::Lkml, StreamId(3), &message("fuck you")).unwrap())
        .await
        .unwrap();

    expect_fatal(&mut wire, Opcode::BrokeUserspace).await;
}

/// `NICE-1.md` §17 and RFC R8: a sender does not choose where its file lands.
#[tokio::test]
async fn a_malicious_filename_cannot_escape_the_download_directory() {
    let mut host = host().await;
    let mut wire = greet(host.address).await;

    let connection = wait(&mut host.events, |event| match event {
        Event::Connected { connection, .. } => Some(*connection),
        _ => None,
    })
    .await;

    let body = b"this should not be where you asked";
    let offer = PullRequest {
        kind: OfferKind::File,
        size: body.len() as u64,
        mime: None,
        name: Some("../../../../tmp/escaped.txt".into()),
        preview: None,
    };
    wire.send(Frame::cbor(Opcode::PullRequest, StreamId(1), &offer).unwrap())
        .await
        .unwrap();

    let stream = wait(&mut host.events, |event| match event {
        Event::Offer { stream, name, .. } => {
            assert_eq!(
                name.as_deref(),
                Some("escaped.txt"),
                "the name shown to the user is already reduced"
            );
            Some(*stream)
        }
        _ => None,
    })
    .await;

    host.handle
        .call(Command::Respond {
            connection,
            stream,
            verdict: Verdict::Merge,
        })
        .await;
    assert_eq!(read(&mut wire).await.unwrap().opcode, Opcode::Merge);

    wire.send(
        Diff {
            offset: 0,
            data: Bytes::from_static(body),
        }
        .into_frame(StreamId(1)),
    )
    .await
    .unwrap();
    wire.send(Frame::bare(Opcode::Done, StreamId(1)))
        .await
        .unwrap();
    wire.send(
        Frame::cbor(
            Opcode::Fsck,
            StreamId(1),
            &nicer_proto::payload::Fsck {
                algorithm: "BLAKE3".into(),
                digest: blake3::hash(body).as_bytes().to_vec(),
                total_size: body.len() as u64,
            },
        )
        .unwrap(),
    )
    .await
    .unwrap();

    assert_eq!(read(&mut wire).await.unwrap().opcode, Opcode::Clean);

    let landed = wait(&mut host.events, |event| match event {
        Event::TransferComplete { path, .. } => Some(path.clone()),
        _ => None,
    })
    .await
    .unwrap();

    assert_eq!(landed.parent().unwrap(), host.downloads);
    assert_eq!(landed.file_name().unwrap(), "escaped.txt");
    assert!(!PathBuf::from("/tmp/escaped.txt").exists());
}

/// RFC R8: a digest that does not match is answered `CORRUPT` and leaves nothing behind.
#[tokio::test]
async fn a_bad_digest_is_corrupt_and_leaves_no_file() {
    let mut host = host().await;
    let mut wire = greet(host.address).await;

    let connection = wait(&mut host.events, |event| match event {
        Event::Connected { connection, .. } => Some(*connection),
        _ => None,
    })
    .await;

    let body = b"tampered";
    let offer = PullRequest {
        kind: OfferKind::File,
        size: body.len() as u64,
        mime: None,
        name: Some("corrupt.bin".into()),
        preview: None,
    };
    wire.send(Frame::cbor(Opcode::PullRequest, StreamId(1), &offer).unwrap())
        .await
        .unwrap();

    let stream = wait(&mut host.events, |event| match event {
        Event::Offer { stream, .. } => Some(*stream),
        _ => None,
    })
    .await;
    host.handle
        .call(Command::Respond {
            connection,
            stream,
            verdict: Verdict::Merge,
        })
        .await;
    assert_eq!(read(&mut wire).await.unwrap().opcode, Opcode::Merge);

    wire.send(
        Diff {
            offset: 0,
            data: Bytes::from_static(body),
        }
        .into_frame(StreamId(1)),
    )
    .await
    .unwrap();
    wire.send(Frame::bare(Opcode::Done, StreamId(1)))
        .await
        .unwrap();
    wire.send(
        Frame::cbor(
            Opcode::Fsck,
            StreamId(1),
            &nicer_proto::payload::Fsck {
                algorithm: "BLAKE3".into(),
                digest: blake3::hash(b"something else").as_bytes().to_vec(),
                total_size: body.len() as u64,
            },
        )
        .unwrap(),
    )
    .await
    .unwrap();

    assert_eq!(read(&mut wire).await.unwrap().opcode, Opcode::Corrupt);

    let entries = std::fs::read_dir(&host.downloads)
        .map(|entries| entries.count())
        .unwrap_or(0);
    assert_eq!(entries, 0, "a corrupt object must leave nothing behind");
    let _ = connection;
}

/// RFC R7: a gap in the `DIFF` sequence cannot be repaired, so it is a protocol error.
#[tokio::test]
async fn a_gap_in_the_diff_sequence_is_broke_userspace() {
    let mut host = host().await;
    let mut wire = greet(host.address).await;

    let connection: ConnectionId = wait(&mut host.events, |event| match event {
        Event::Connected { connection, .. } => Some(*connection),
        _ => None,
    })
    .await;

    let offer = PullRequest {
        kind: OfferKind::File,
        size: 16,
        mime: None,
        name: Some("sparse.bin".into()),
        preview: None,
    };
    wire.send(Frame::cbor(Opcode::PullRequest, StreamId(1), &offer).unwrap())
        .await
        .unwrap();

    let stream = wait(&mut host.events, |event| match event {
        Event::Offer { stream, .. } => Some(*stream),
        _ => None,
    })
    .await;
    host.handle
        .call(Command::Respond {
            connection,
            stream,
            verdict: Verdict::Merge,
        })
        .await;
    assert_eq!(read(&mut wire).await.unwrap().opcode, Opcode::Merge);

    wire.send(
        Diff {
            offset: 8,
            data: Bytes::from_static(b"12345678"),
        }
        .into_frame(StreamId(1)),
    )
    .await
    .unwrap();

    expect_fatal(&mut wire, Opcode::BrokeUserspace).await;
}

/// RFC R11: a header without its payload cannot hold a connection and its buffer open.
#[tokio::test]
async fn a_frame_that_stalls_part_way_through_is_closed() {
    let host = host_with(|config| config.timeouts.frame_secs = 1).await;
    let mut wire = greet(host.address).await;

    let mut header = [
        MAGIC,
        0x01,
        Opcode::Lkml as u8,
        0x00,
        0,
        0,
        0,
        1,
        0,
        0,
        0,
        0,
    ];
    header[8..12].copy_from_slice(&4096u32.to_be_bytes());
    wire.get_mut().write_all(&header).await.unwrap();
    wire.get_mut()
        .write_all(b"a partial payload")
        .await
        .unwrap();

    assert!(
        read(&mut wire).await.is_none(),
        "a stalled frame must end the connection"
    );
}

#[test]
fn a_sender_never_chooses_a_path() {
    assert_eq!(sanitize_filename("../../etc/passwd"), "passwd");
    assert_eq!(sanitize_filename("/etc/shadow"), "shadow");
    assert_eq!(
        sanitize_filename(r"C:\Windows\System32\evil.dll"),
        "evil.dll"
    );
    assert_eq!(sanitize_filename(".."), "received");
    assert_eq!(sanitize_filename("."), "received");
    assert_eq!(sanitize_filename(""), "received");
    assert_eq!(sanitize_filename("   "), "received");
    assert_eq!(sanitize_filename("nul\0byte.txt"), "nulbyte.txt");
    assert_eq!(
        sanitize_filename("carriage\r\nreturn.txt"),
        "carriagereturn.txt"
    );
    assert_eq!(sanitize_filename("cat.png"), "cat.png");
    assert!(sanitize_filename(&"a".repeat(5000)).len() <= 200);
}

#[test]
fn a_colliding_name_is_disambiguated_never_overwritten() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("cat.png"), b"first").unwrap();

    let next = unique_path(dir.path(), "cat.png");
    assert_eq!(next.file_name().unwrap(), "cat (2).png");

    std::fs::write(&next, b"second").unwrap();
    assert_eq!(
        unique_path(dir.path(), "cat.png").file_name().unwrap(),
        "cat (3).png"
    );
    assert_eq!(
        std::fs::read(dir.path().join("cat.png")).unwrap(),
        b"first",
        "the original must survive"
    );
}

#[test]
fn a_dotfile_keeps_its_whole_name() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(
        unique_path(dir.path(), "config").file_name().unwrap(),
        "config"
    );
}
