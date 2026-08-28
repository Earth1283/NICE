use std::collections::BTreeMap;

use nicer::event::{Command, ConnectionId, ConnectionSummary, Direction, Event, Status, Verdict};
use nicer::identity::Fingerprint;
use nicer::node::Reply;
use nicer::pairing::PairedPeer;
use nicer_proto::payload::{OfferKind, ShutUpScope, TransportMode};

fn fingerprint() -> Fingerprint {
    "9f3a1c04b7e2d580000000000000000000000000000000000000000000000000"
        .parse()
        .unwrap()
}

fn peer() -> PairedPeer {
    PairedPeer {
        fingerprint: fingerprint(),
        device: "thinkpad".into(),
        paired_at: 1,
        last_seen: None,
        last_address: None,
        auto_accept_files: false,
        auto_accept_clipboard: false,
    }
}

fn status() -> Status {
    Status {
        device: "alpha".into(),
        fingerprint: fingerprint(),
        short: fingerprint().short(),
        listen: "127.0.0.1:6969".parse().unwrap(),
        transport: TransportMode::Secure,
        discovery: true,
        download_dir: "/tmp/nicer".into(),
        connections: 1,
        paired_peers: 2,
    }
}

fn connection() -> ConnectionSummary {
    ConnectionSummary {
        connection: ConnectionId(1),
        address: "127.0.0.1:6969".parse().unwrap(),
        device: "beta".into(),
        transport: TransportMode::Secure,
        direction: Direction::Outgoing,
        fingerprint: Some(fingerprint()),
        paired: true,
    }
}

/// An internally tagged enum cannot carry a sequence in a newtype variant. Every reply is
/// checked here rather than discovered by a client that waits forever for an answer.
#[test]
fn every_reply_serialises_with_its_tag() {
    let replies = vec![
        Reply::Ok,
        Reply::Status(status()),
        Reply::Peers {
            peers: vec![peer()],
        },
        Reply::Connections {
            connections: vec![connection()],
        },
        Reply::Connected {
            connection: ConnectionId(7),
        },
        Reply::Error {
            message: "no".into(),
        },
    ];

    for reply in replies {
        let json = serde_json::to_value(&reply)
            .unwrap_or_else(|e| panic!("{reply:?} does not serialise: {e}"));
        assert!(
            json.get("reply").and_then(|tag| tag.as_str()).is_some(),
            "{json} has no reply tag"
        );
    }
}

#[test]
fn every_event_serialises_with_its_tag() {
    let events = vec![
        Event::Listening {
            address: "127.0.0.1:6969".parse().unwrap(),
            transport: TransportMode::Secure,
            device: "alpha".into(),
            fingerprint: fingerprint(),
        },
        Event::PeerDiscovered {
            instance: "beta".into(),
            device: "beta".into(),
            addresses: vec!["192.168.1.42".parse().unwrap()],
            port: 6969,
            secure: true,
            fingerprint: Some(fingerprint()),
        },
        Event::PeerLost {
            instance: "beta".into(),
        },
        Event::Connected {
            connection: ConnectionId(1),
            address: "127.0.0.1:6969".parse().unwrap(),
            device: "beta".into(),
            transport: TransportMode::Secure,
            direction: Direction::Incoming,
            fingerprint: Some(fingerprint()),
            paired: false,
        },
        Event::Disconnected {
            connection: ConnectionId(1),
            reason: "peer closed the connection".into(),
        },
        Event::PairingRequired {
            connection: ConnectionId(1),
            address: "127.0.0.1:6969".parse().unwrap(),
            device: "beta".into(),
            fingerprint: fingerprint(),
            short: fingerprint().short(),
        },
        Event::IdentityChanged {
            address: "127.0.0.1:6969".parse().unwrap(),
            expected: fingerprint(),
            reason: "identity changed".into(),
        },
        Event::Offer {
            connection: ConnectionId(1),
            stream: 1,
            kind: OfferKind::File,
            size: 42069,
            name: Some("shitpost.png".into()),
            mime: Some("image/png".into()),
            preview: None,
            auto_accepted: false,
        },
        Event::OfferResolved {
            connection: ConnectionId(1),
            stream: 1,
            direction: Direction::Incoming,
            verdict: Verdict::Merge,
        },
        Event::TransferProgress {
            connection: ConnectionId(1),
            stream: 1,
            direction: Direction::Outgoing,
            transferred: 16384,
            total: 42069,
        },
        Event::TransferComplete {
            connection: ConnectionId(1),
            stream: 1,
            direction: Direction::Incoming,
            kind: OfferKind::File,
            path: Some("/tmp/cat.png".into()),
        },
        Event::TransferFailed {
            connection: ConnectionId(1),
            stream: 1,
            direction: Direction::Incoming,
            reason: "BLAKE3 digest did not match".into(),
        },
        Event::Chat {
            connection: ConnectionId(1),
            stream: 3,
            from: "192.168.1.14".parse().unwrap(),
            fingerprint: Some(fingerprint()),
            message_id: "1-1".into(),
            timestamp: 0,
            text: "this meme sucks".into(),
            tags: Some(BTreeMap::from([("Acked-by".into(), "torvalds".into())])),
        },
        Event::RateLimited {
            connection: ConnectionId(1),
            direction: Direction::Outgoing,
            retry_after_ms: 5000,
            scope: ShutUpScope::Kind,
        },
        Event::ProtocolError {
            connection: ConnectionId(1),
            opcode: "CPP".into(),
            reason: "malformed payload".into(),
        },
        Event::Resynced {
            connection: ConnectionId(1),
        },
        Event::Warning {
            message: "no system clipboard".into(),
        },
    ];

    for event in events {
        let json = serde_json::to_value(&event)
            .unwrap_or_else(|e| panic!("{event:?} does not serialise: {e}"));
        assert!(
            json.get("event").and_then(|tag| tag.as_str()).is_some(),
            "{json} has no event tag"
        );
    }
}

#[test]
fn commands_parse_from_the_documented_json() {
    let cases = [
        r#"{"cmd":"status"}"#,
        r#"{"cmd":"list_peers"}"#,
        r#"{"cmd":"list_connections"}"#,
        r#"{"cmd":"connect","address":"192.168.1.42:6969"}"#,
        r#"{"cmd":"disconnect","connection":1}"#,
        r#"{"cmd":"pair","fingerprint":"9f3a1c04b7e2d580000000000000000000000000000000000000000000000000"}"#,
        r#"{"cmd":"unpair","fingerprint":"9f3a1c04b7e2d580000000000000000000000000000000000000000000000000"}"#,
        r#"{"cmd":"auto_accept","fingerprint":"9f3a1c04b7e2d580000000000000000000000000000000000000000000000000","kind":"file","enabled":true}"#,
        r#"{"cmd":"send_file","connection":1,"path":"/tmp/cat.png"}"#,
        r#"{"cmd":"send_clipboard","connection":1}"#,
        r#"{"cmd":"send_clipboard","connection":1,"text":"look at this..."}"#,
        r#"{"cmd":"respond","connection":1,"stream":1,"verdict":"merge"}"#,
        r#"{"cmd":"respond","connection":1,"stream":1,"verdict":"fuck_off","reason":"no"}"#,
        r#"{"cmd":"respond","connection":1,"stream":1,"verdict":"big_diff","max_size":1024}"#,
        r#"{"cmd":"chat","connection":1,"text":"merged"}"#,
        r#"{"cmd":"resync","connection":1}"#,
    ];

    for case in cases {
        serde_json::from_str::<Command>(case)
            .unwrap_or_else(|e| panic!("{case} does not parse: {e}"));
    }
}

/// The `id` a client attaches is echoed back so replies can be matched to commands.
#[test]
fn an_envelope_carries_an_id_alongside_the_command() {
    #[derive(serde::Deserialize)]
    struct Envelope {
        id: Option<serde_json::Value>,
        #[serde(flatten)]
        command: Command,
    }

    let envelope: Envelope =
        serde_json::from_str(r#"{"id":42,"cmd":"chat","connection":1,"text":"merged"}"#).unwrap();
    assert_eq!(envelope.id, Some(serde_json::json!(42)));
    assert!(matches!(envelope.command, Command::Chat { .. }));
}

#[test]
fn a_fingerprint_survives_json_and_display() {
    let text = serde_json::to_string(&fingerprint()).unwrap();
    assert_eq!(text, format!("\"{}\"", fingerprint()));
    assert_eq!(
        serde_json::from_str::<Fingerprint>(&text).unwrap(),
        fingerprint()
    );
    assert_eq!(fingerprint().short(), "9f3a1c04 b7e2d580");
    assert!("nonsense".parse::<Fingerprint>().is_err());
    assert!("9f3a".parse::<Fingerprint>().is_err());
}
