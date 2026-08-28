use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;

use nicer::event::{ConnectionId, Direction};
use nicer::identity::Fingerprint;
use nicer_proto::payload::OfferKind;
use ratatui::style::Color;

pub fn offer_kind_label(kind: OfferKind) -> &'static str {
    match kind {
        OfferKind::File => "file",
        OfferKind::Clipboard => "clipboard",
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tab {
    Peers,
    Connections,
    Transfers,
    Chat,
    Log,
}

impl Tab {
    pub const ALL: [Tab; 5] = [
        Tab::Peers,
        Tab::Connections,
        Tab::Transfers,
        Tab::Chat,
        Tab::Log,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Tab::Peers => "Peers",
            Tab::Connections => "Connections",
            Tab::Transfers => "Transfers",
            Tab::Chat => "Chat",
            Tab::Log => "Log",
        }
    }

    fn index(self) -> usize {
        Tab::ALL.iter().position(|t| *t == self).unwrap()
    }

    pub fn next(self) -> Tab {
        Tab::ALL[(self.index() + 1) % Tab::ALL.len()]
    }

    pub fn prev(self) -> Tab {
        Tab::ALL[(self.index() + Tab::ALL.len() - 1) % Tab::ALL.len()]
    }

    pub fn from_digit(d: u8) -> Option<Tab> {
        Tab::ALL.get(usize::from(d).checked_sub(1)?).copied()
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PeersPane {
    Paired,
    Discovered,
}

pub struct Discovered {
    pub device: String,
    pub addresses: Vec<IpAddr>,
    pub port: u16,
    pub secure: bool,
    pub fingerprint: Option<Fingerprint>,
}

pub enum TransferStatus {
    /// An incoming `PULL_REQUEST` nobody has answered yet.
    Offered {
        kind: OfferKind,
        size: u64,
        mime: Option<String>,
        preview: Option<String>,
    },
    /// `MERGE` seen; `DIFF` frames have not started arriving yet.
    Accepted { kind: OfferKind },
    Active {
        direction: Direction,
        kind: OfferKind,
        transferred: u64,
        total: u64,
    },
    Complete {
        direction: Direction,
        kind: OfferKind,
        path: Option<PathBuf>,
    },
    Failed {
        direction: Direction,
        reason: String,
    },
    Refused {
        direction: Direction,
        reason: Option<String>,
    },
    TooBig {
        direction: Direction,
        max_size: Option<u64>,
    },
}

pub struct TransferEntry {
    pub connection: ConnectionId,
    pub stream: u32,
    pub name: Option<String>,
    pub status: TransferStatus,
    pub seq: u64,
}

impl TransferEntry {
    pub fn kind(&self) -> Option<OfferKind> {
        match &self.status {
            TransferStatus::Offered { kind, .. }
            | TransferStatus::Accepted { kind }
            | TransferStatus::Active { kind, .. }
            | TransferStatus::Complete { kind, .. } => Some(*kind),
            _ => None,
        }
    }

    pub fn is_pending_response(&self) -> bool {
        matches!(self.status, TransferStatus::Offered { .. })
    }
}

pub struct ChatLine {
    pub from_us: bool,
    pub from: String,
    pub fingerprint: Option<Fingerprint>,
    pub text: String,
}

pub struct LogLine {
    pub color: Color,
    pub text: String,
}

#[derive(Clone)]
pub struct PairingPrompt {
    pub address: SocketAddr,
    pub device: String,
    pub fingerprint: Fingerprint,
    pub short: String,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RequestKind {
    Status,
    ListPeers,
    ListConnections,
    Connect,
    Disconnect,
    Pair,
    Unpair,
    AutoAccept,
    SendFile,
    SendClipboard,
    Respond,
    Chat,
    Resync,
}

impl RequestKind {
    pub fn label(self) -> &'static str {
        match self {
            RequestKind::Status => "status",
            RequestKind::ListPeers => "list_peers",
            RequestKind::ListConnections => "list_connections",
            RequestKind::Connect => "connect",
            RequestKind::Disconnect => "disconnect",
            RequestKind::Pair => "pair",
            RequestKind::Unpair => "unpair",
            RequestKind::AutoAccept => "auto-accept",
            RequestKind::SendFile => "send file",
            RequestKind::SendClipboard => "send clipboard",
            RequestKind::Respond => "respond",
            RequestKind::Chat => "chat",
            RequestKind::Resync => "resync",
        }
    }
}

pub enum Modal {
    None,
    Help,
    Connect,
    SendFile { connection: ConnectionId },
    SendClipboardText { connection: ConnectionId },
    ChatInput { connection: ConnectionId },
    BigDiffSize { connection: ConnectionId, stream: u32 },
    FuckOffReason { connection: ConnectionId, stream: u32 },
    ConfirmUnpair { fingerprint: Fingerprint, device: String },
    Pairing(PairingPrompt),
    Offer { connection: ConnectionId, stream: u32 },
}
