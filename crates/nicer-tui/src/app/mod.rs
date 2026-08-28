mod events;
mod keys;
mod modal;
mod types;

pub use types::*;

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::net::SocketAddr;

use nicer::event::{Command, ConnectionId, ConnectionSummary, Verdict};
use nicer::identity::Fingerprint;
use nicer::node::{NodeHandle, Reply};
use nicer::pairing::PairedPeer;
use nicer_proto::payload::OfferKind;
use ratatui::style::Color;
use tokio::sync::mpsc;

use crate::theme;
use crate::wire::{tick_for, Tick};

const LOG_CAP: usize = 500;
const TICKER_CAP: usize = 64;
const TRANSFER_CAP: usize = 300;

pub struct App {
    pub handle: NodeHandle,
    results_tx: mpsc::UnboundedSender<(RequestKind, Reply)>,

    pub device: String,
    pub fingerprint: Option<Fingerprint>,
    pub listen: Option<SocketAddr>,
    pub status: Option<nicer::event::Status>,

    pub peers: Vec<PairedPeer>,
    pub discovered: BTreeMap<String, Discovered>,
    pub connections: BTreeMap<ConnectionId, ConnectionSummary>,

    pub transfers: Vec<TransferEntry>,
    transfer_index: HashMap<(ConnectionId, u32), usize>,
    transfer_seq: u64,
    pending_sends: HashMap<ConnectionId, VecDeque<(OfferKind, Option<String>)>>,

    pending_pairing_popups: VecDeque<ConnectionId>,
    pending_offer_popups: VecDeque<(ConnectionId, u32)>,

    pub chats: BTreeMap<ConnectionId, Vec<ChatLine>>,
    pub log: VecDeque<LogLine>,
    pub ticker: VecDeque<Tick>,

    pub tab: Tab,
    pub peers_pane: PeersPane,
    pub peers_index: usize,
    pub discovered_index: usize,
    pub connections_index: usize,
    pub transfers_index: usize,
    pub chat_connection: Option<ConnectionId>,
    pub chat_scroll: u16,
    pub log_scroll: u16,

    pub modal: Modal,
    pub input: String,

    pub status_msg: Option<(String, Color)>,
    pub should_quit: bool,
}

impl App {
    pub fn new(handle: NodeHandle, results_tx: mpsc::UnboundedSender<(RequestKind, Reply)>) -> Self {
        App {
            handle,
            results_tx,
            device: String::new(),
            fingerprint: None,
            listen: None,
            status: None,
            peers: Vec::new(),
            discovered: BTreeMap::new(),
            connections: BTreeMap::new(),
            transfers: Vec::new(),
            transfer_index: HashMap::new(),
            transfer_seq: 0,
            pending_sends: HashMap::new(),
            pending_pairing_popups: VecDeque::new(),
            pending_offer_popups: VecDeque::new(),
            chats: BTreeMap::new(),
            log: VecDeque::new(),
            ticker: VecDeque::new(),
            tab: Tab::Peers,
            peers_pane: PeersPane::Paired,
            peers_index: 0,
            discovered_index: 0,
            connections_index: 0,
            transfers_index: 0,
            chat_connection: None,
            chat_scroll: 0,
            log_scroll: 0,
            modal: Modal::None,
            input: String::new(),
            status_msg: None,
            should_quit: false,
        }
    }

    pub fn set_status(&mut self, text: impl Into<String>, color: Color) {
        self.status_msg = Some((text.into(), color));
    }

    pub fn dispatch(&self, kind: RequestKind, command: Command) {
        let handle = self.handle.clone();
        let tx = self.results_tx.clone();
        tokio::spawn(async move {
            let reply = handle.call(command).await;
            let _ = tx.send((kind, reply));
        });
    }

    fn refresh_peers(&self) {
        self.dispatch(RequestKind::ListPeers, Command::ListPeers);
    }

    fn refresh_connections(&self) {
        self.dispatch(RequestKind::ListConnections, Command::ListConnections);
    }

    pub fn on_reply(&mut self, kind: RequestKind, reply: Reply) {
        match reply {
            Reply::Error { message } => self.set_status(format!("{}: {message}", kind.label()), theme::CUT),
            Reply::Ok => {
                self.set_status(format!("{} ok", kind.label()), theme::NICE);
                match kind {
                    RequestKind::Pair | RequestKind::Unpair | RequestKind::AutoAccept => {
                        self.refresh_peers()
                    }
                    RequestKind::Disconnect | RequestKind::Resync => self.refresh_connections(),
                    _ => {}
                }
            }
            Reply::Status(status) => {
                self.listen = Some(status.listen);
                self.device = status.device.clone();
                self.fingerprint = Some(status.fingerprint);
                self.status = Some(status);
            }
            Reply::Peers { mut peers } => {
                peers.sort_by(|a, b| a.device.cmp(&b.device));
                self.peers = peers;
                if self.peers_index >= self.peers.len() {
                    self.peers_index = self.peers.len().saturating_sub(1);
                }
            }
            Reply::Connections { connections } => {
                self.connections = connections.into_iter().map(|c| (c.connection, c)).collect();
                if self.connections_index >= self.connections.len() {
                    self.connections_index = self.connections.len().saturating_sub(1);
                }
            }
            Reply::Connected { connection } => {
                self.set_status(format!("connected: {connection}"), theme::NICE);
            }
        }
    }

    fn push_tick(&mut self, event: &nicer::event::Event) {
        if let Some(t) = tick_for(event) {
            self.ticker.push_front(t);
            while self.ticker.len() > TICKER_CAP {
                self.ticker.pop_back();
            }
        }
    }

    fn push_log(&mut self, text: impl Into<String>, color: Color) {
        self.log.push_back(LogLine {
            color,
            text: text.into(),
        });
        while self.log.len() > LOG_CAP {
            self.log.pop_front();
        }
    }

    fn known_or_pending(&mut self, connection: ConnectionId, stream: u32) -> (Option<OfferKind>, Option<String>) {
        if let Some(&idx) = self.transfer_index.get(&(connection, stream)) {
            let e = &self.transfers[idx];
            (e.kind(), e.name.clone())
        } else if let Some(q) = self.pending_sends.get_mut(&connection) {
            q.pop_front().map(|(k, n)| (Some(k), n)).unwrap_or((None, None))
        } else {
            (None, None)
        }
    }

    fn upsert_transfer(
        &mut self,
        connection: ConnectionId,
        stream: u32,
        name_hint: Option<String>,
        status: TransferStatus,
    ) {
        self.transfer_seq += 1;
        let seq = self.transfer_seq;
        if let Some(&idx) = self.transfer_index.get(&(connection, stream)) {
            let entry = &mut self.transfers[idx];
            if entry.name.is_none() {
                entry.name = name_hint;
            }
            entry.status = status;
            entry.seq = seq;
        } else {
            self.transfers.push(TransferEntry {
                connection,
                stream,
                name: name_hint,
                status,
                seq,
            });
            self.transfer_index
                .insert((connection, stream), self.transfers.len() - 1);
        }
        if self.transfers.len() > TRANSFER_CAP {
            if let Some((min_i, _)) = self.transfers.iter().enumerate().min_by_key(|(_, e)| e.seq) {
                self.transfers.remove(min_i);
                self.transfer_index.clear();
                for (i, e) in self.transfers.iter().enumerate() {
                    self.transfer_index.insert((e.connection, e.stream), i);
                }
            }
        }
    }

    fn resolve_offer(&mut self, connection: ConnectionId, stream: u32, verdict: Verdict) {
        self.pending_offer_popups.retain(|k| *k != (connection, stream));
        let (kind, name) = self.known_or_pending(connection, stream);
        let status = match verdict {
            Verdict::Merge => TransferStatus::Accepted {
                kind: kind.unwrap_or(OfferKind::File),
            },
            Verdict::FuckOff { reason } => TransferStatus::Refused {
                direction: nicer::event::Direction::Outgoing,
                reason,
            },
            Verdict::BigDiff { max_size } => TransferStatus::TooBig {
                direction: nicer::event::Direction::Outgoing,
                max_size,
            },
        };
        self.upsert_transfer(connection, stream, name, status);
    }

    pub fn connection_label(&self, id: ConnectionId) -> String {
        match self.connections.get(&id) {
            Some(c) => format!("{} ({})", c.device, c.address),
            None => format!("#{id}"),
        }
    }

    pub fn ordered_transfers(&self) -> Vec<&TransferEntry> {
        let mut v: Vec<&TransferEntry> = self.transfers.iter().collect();
        v.sort_by_key(|e| std::cmp::Reverse(e.seq));
        v
    }

    pub fn on_lagged(&mut self, skipped: u64) {
        self.push_log(format!("{skipped} events were dropped; re-reading state"), theme::CUT);
        self.set_status(format!("fell behind by {skipped} events; resyncing"), theme::CUT);
        self.refresh_peers();
        self.refresh_connections();
        self.dispatch(RequestKind::Status, Command::Status);
    }

    fn close_modal(&mut self) {
        self.modal = Modal::None;
        self.input.clear();
    }

    fn maybe_pop_pairing_popup(&mut self) {
        if matches!(self.modal, Modal::None) {
            if let Some(connection) = self.pending_pairing_popups.pop_front() {
                if let Some(c) = self.connections.get(&connection) {
                    if let Some(fingerprint) = c.fingerprint {
                        self.modal = Modal::Pairing(PairingPrompt {
                            address: c.address,
                            device: c.device.clone(),
                            fingerprint,
                            short: fingerprint.short(),
                        });
                    }
                }
            } else if let Some((connection, stream)) = self.pending_offer_popups.pop_front() {
                self.modal = Modal::Offer { connection, stream };
            }
        }
    }
}
