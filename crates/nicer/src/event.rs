use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;

use nicer_proto::payload::{OfferKind, ShutUpScope, TransportMode};
use serde::{Deserialize, Serialize};

use crate::identity::Fingerprint;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ConnectionId(pub u64);

impl std::fmt::Display for ConnectionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "verdict", rename_all = "snake_case")]
pub enum Verdict {
    Merge,
    FuckOff {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
    BigDiff {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_size: Option<u64>,
    },
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    Incoming,
    Outgoing,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum Command {
    Status,
    ListPeers,
    ListConnections,
    Connect {
        address: SocketAddr,
    },
    Disconnect {
        connection: ConnectionId,
    },
    Pair {
        fingerprint: Fingerprint,
        #[serde(default)]
        device: Option<String>,
    },
    Unpair {
        fingerprint: Fingerprint,
    },
    AutoAccept {
        fingerprint: Fingerprint,
        kind: OfferKind,
        enabled: bool,
    },
    SendFile {
        connection: ConnectionId,
        path: PathBuf,
    },
    SendClipboard {
        connection: ConnectionId,
        #[serde(default)]
        text: Option<String>,
    },
    Respond {
        connection: ConnectionId,
        stream: u32,
        #[serde(flatten)]
        verdict: Verdict,
    },
    Chat {
        connection: ConnectionId,
        text: String,
    },
    Resync {
        connection: ConnectionId,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
    Listening {
        address: SocketAddr,
        transport: TransportMode,
        device: String,
        fingerprint: Fingerprint,
    },
    PeerDiscovered {
        instance: String,
        device: String,
        addresses: Vec<IpAddr>,
        port: u16,
        secure: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        fingerprint: Option<Fingerprint>,
    },
    PeerLost {
        instance: String,
    },
    Connected {
        connection: ConnectionId,
        address: SocketAddr,
        device: String,
        transport: TransportMode,
        direction: Direction,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        fingerprint: Option<Fingerprint>,
        paired: bool,
    },
    Disconnected {
        connection: ConnectionId,
        reason: String,
    },
    /// A peer this node has never paired with. Nothing is stored until the user says so.
    PairingRequired {
        connection: ConnectionId,
        address: SocketAddr,
        device: String,
        fingerprint: Fingerprint,
        short: String,
    },
    IdentityChanged {
        address: SocketAddr,
        expected: Fingerprint,
        reason: String,
    },
    Offer {
        connection: ConnectionId,
        stream: u32,
        kind: OfferKind,
        size: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mime: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        preview: Option<String>,
        auto_accepted: bool,
    },
    OfferResolved {
        connection: ConnectionId,
        stream: u32,
        direction: Direction,
        #[serde(flatten)]
        verdict: Verdict,
    },
    TransferProgress {
        connection: ConnectionId,
        stream: u32,
        direction: Direction,
        transferred: u64,
        total: u64,
    },
    TransferComplete {
        connection: ConnectionId,
        stream: u32,
        direction: Direction,
        kind: OfferKind,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path: Option<PathBuf>,
    },
    TransferFailed {
        connection: ConnectionId,
        stream: u32,
        direction: Direction,
        reason: String,
    },
    Chat {
        connection: ConnectionId,
        stream: u32,
        from: IpAddr,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        fingerprint: Option<Fingerprint>,
        message_id: String,
        timestamp: u64,
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tags: Option<std::collections::BTreeMap<String, String>>,
    },
    RateLimited {
        connection: ConnectionId,
        direction: Direction,
        retry_after_ms: u64,
        scope: ShutUpScope,
    },
    ProtocolError {
        connection: ConnectionId,
        opcode: String,
        reason: String,
    },
    Resynced {
        connection: ConnectionId,
    },
    Warning {
        message: String,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConnectionSummary {
    pub connection: ConnectionId,
    pub address: SocketAddr,
    pub device: String,
    pub transport: TransportMode,
    pub direction: Direction,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fingerprint: Option<Fingerprint>,
    pub paired: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Status {
    pub device: String,
    pub fingerprint: Fingerprint,
    pub short: String,
    pub listen: SocketAddr,
    pub transport: TransportMode,
    pub discovery: bool,
    pub download_dir: PathBuf,
    pub connections: usize,
    pub paired_peers: usize,
}
