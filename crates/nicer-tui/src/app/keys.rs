use std::net::SocketAddr;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use nicer::event::{Command, ConnectionId};
use nicer_proto::payload::OfferKind;

use super::{App, Modal, PairingPrompt, PeersPane, RequestKind, Tab};
use crate::theme;

const DEFAULT_PORT: u16 = 6969;

impl App {
    pub fn on_key(&mut self, key: KeyEvent) {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.should_quit = true;
            return;
        }

        if !matches!(self.modal, Modal::None) {
            self.on_modal_key(key);
            return;
        }

        match key.code {
            KeyCode::Char('q') => {
                self.should_quit = true;
                return;
            }
            KeyCode::Char('?') => {
                self.modal = Modal::Help;
                return;
            }
            KeyCode::Tab => {
                self.tab = self.tab.next();
                return;
            }
            KeyCode::BackTab => {
                self.tab = self.tab.prev();
                return;
            }
            KeyCode::Char(c @ '1'..='5') => {
                if let Some(t) = Tab::from_digit(c as u8 - b'0') {
                    self.tab = t;
                }
                return;
            }
            KeyCode::Char('n') => {
                self.modal = Modal::Connect;
                return;
            }
            _ => {}
        }

        match self.tab {
            Tab::Peers => self.on_key_peers(key),
            Tab::Connections => self.on_key_connections(key),
            Tab::Transfers => self.on_key_transfers(key),
            Tab::Chat => self.on_key_chat(key),
            Tab::Log => self.on_key_log(key),
        }
    }

    fn on_key_peers(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Left | KeyCode::Char('h') => self.peers_pane = PeersPane::Paired,
            KeyCode::Right | KeyCode::Char('l') => self.peers_pane = PeersPane::Discovered,
            KeyCode::Up | KeyCode::Char('k') => match self.peers_pane {
                PeersPane::Paired => self.peers_index = self.peers_index.saturating_sub(1),
                PeersPane::Discovered => self.discovered_index = self.discovered_index.saturating_sub(1),
            },
            KeyCode::Down | KeyCode::Char('j') => match self.peers_pane {
                PeersPane::Paired => {
                    if !self.peers.is_empty() {
                        self.peers_index = (self.peers_index + 1).min(self.peers.len() - 1);
                    }
                }
                PeersPane::Discovered => {
                    if !self.discovered.is_empty() {
                        self.discovered_index = (self.discovered_index + 1).min(self.discovered.len() - 1);
                    }
                }
            },
            KeyCode::Enter => match self.peers_pane {
                PeersPane::Paired => {
                    if let Some(p) = self.peers.get(self.peers_index) {
                        match p.last_address {
                            Some(ip) => {
                                let address = SocketAddr::new(ip, DEFAULT_PORT);
                                self.set_status(
                                    format!("dialling {address} (last-known, default port)"),
                                    theme::SIGNAL,
                                );
                                self.dispatch(RequestKind::Connect, Command::Connect { address });
                            }
                            None => self.set_status(
                                "no known address for this peer; press 'n' to connect manually",
                                theme::WIRE,
                            ),
                        }
                    }
                }
                PeersPane::Discovered => {
                    if let Some((_, d)) = self.discovered.iter().nth(self.discovered_index) {
                        if let Some(ip) = d.addresses.first() {
                            let address = SocketAddr::new(*ip, d.port);
                            self.dispatch(RequestKind::Connect, Command::Connect { address });
                        }
                    }
                }
            },
            KeyCode::Char('u') if self.peers_pane == PeersPane::Paired => {
                if let Some(p) = self.peers.get(self.peers_index) {
                    self.modal = Modal::ConfirmUnpair {
                        fingerprint: p.fingerprint,
                        device: p.device.clone(),
                    };
                }
            }
            KeyCode::Char('f') if self.peers_pane == PeersPane::Paired => {
                if let Some(p) = self.peers.get(self.peers_index) {
                    self.dispatch(
                        RequestKind::AutoAccept,
                        Command::AutoAccept {
                            fingerprint: p.fingerprint,
                            kind: OfferKind::File,
                            enabled: !p.auto_accept_files,
                        },
                    );
                }
            }
            KeyCode::Char('c') if self.peers_pane == PeersPane::Paired => {
                if let Some(p) = self.peers.get(self.peers_index) {
                    self.dispatch(
                        RequestKind::AutoAccept,
                        Command::AutoAccept {
                            fingerprint: p.fingerprint,
                            kind: OfferKind::Clipboard,
                            enabled: !p.auto_accept_clipboard,
                        },
                    );
                }
            }
            _ => {}
        }
    }

    fn selected_connection(&self) -> Option<ConnectionId> {
        self.connections.keys().nth(self.connections_index).copied()
    }

    fn on_key_connections(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => self.connections_index = self.connections_index.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => {
                if !self.connections.is_empty() {
                    self.connections_index = (self.connections_index + 1).min(self.connections.len() - 1);
                }
            }
            KeyCode::Char('p') => {
                if let Some(id) = self.selected_connection() {
                    if let Some(c) = self.connections.get(&id) {
                        if let Some(fingerprint) = c.fingerprint {
                            if !c.paired {
                                self.modal = Modal::Pairing(PairingPrompt {
                                    address: c.address,
                                    device: c.device.clone(),
                                    fingerprint,
                                    short: fingerprint.short(),
                                });
                            } else {
                                self.set_status("already paired", theme::NICE);
                            }
                        } else {
                            self.set_status("PLAINTEXT connection: no identity to pair", theme::WIRE);
                        }
                    }
                }
            }
            KeyCode::Char('d') => {
                if let Some(connection) = self.selected_connection() {
                    self.dispatch(RequestKind::Disconnect, Command::Disconnect { connection });
                }
            }
            KeyCode::Char('s') => {
                if let Some(connection) = self.selected_connection() {
                    self.dispatch(RequestKind::Resync, Command::Resync { connection });
                }
            }
            KeyCode::Char('f') => {
                if let Some(connection) = self.selected_connection() {
                    self.modal = Modal::SendFile { connection };
                }
            }
            KeyCode::Char('v') => {
                if let Some(connection) = self.selected_connection() {
                    self.pending_sends
                        .entry(connection)
                        .or_default()
                        .push_back((OfferKind::Clipboard, None));
                    self.dispatch(
                        RequestKind::SendClipboard,
                        Command::SendClipboard { connection, text: None },
                    );
                }
            }
            KeyCode::Char('V') => {
                if let Some(connection) = self.selected_connection() {
                    self.modal = Modal::SendClipboardText { connection };
                }
            }
            KeyCode::Char('g') => {
                if let Some(connection) = self.selected_connection() {
                    self.chat_connection = Some(connection);
                    self.tab = Tab::Chat;
                }
            }
            _ => {}
        }
    }

    fn on_key_transfers(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => self.transfers_index = self.transfers_index.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => {
                if !self.transfers.is_empty() {
                    self.transfers_index = (self.transfers_index + 1).min(self.transfers.len() - 1);
                }
            }
            KeyCode::Enter => {
                if let Some(entry) = self.ordered_transfers().get(self.transfers_index) {
                    if entry.is_pending_response() {
                        self.modal = Modal::Offer {
                            connection: entry.connection,
                            stream: entry.stream,
                        };
                    }
                }
            }
            _ => {}
        }
    }

    fn chat_connections(&self) -> Vec<ConnectionId> {
        self.connections.keys().copied().collect()
    }

    fn on_key_chat(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Left | KeyCode::Char('h') => {
                let ids = self.chat_connections();
                if let Some(cur) = self.chat_connection {
                    if let Some(pos) = ids.iter().position(|i| *i == cur) {
                        let new_pos = (pos + ids.len() - 1) % ids.len();
                        self.chat_connection = ids.get(new_pos).copied();
                    }
                } else {
                    self.chat_connection = ids.first().copied();
                }
            }
            KeyCode::Right | KeyCode::Char('l') => {
                let ids = self.chat_connections();
                if let Some(cur) = self.chat_connection {
                    if let Some(pos) = ids.iter().position(|i| *i == cur) {
                        let new_pos = (pos + 1) % ids.len();
                        self.chat_connection = ids.get(new_pos).copied();
                    }
                } else {
                    self.chat_connection = ids.first().copied();
                }
            }
            KeyCode::Up | KeyCode::Char('k') => self.chat_scroll = self.chat_scroll.saturating_add(1),
            KeyCode::Down | KeyCode::Char('j') => self.chat_scroll = self.chat_scroll.saturating_sub(1),
            KeyCode::Char('i') | KeyCode::Enter => {
                if let Some(connection) = self.chat_connection {
                    self.modal = Modal::ChatInput { connection };
                } else {
                    self.set_status("open a connection first", theme::WIRE);
                }
            }
            _ => {}
        }
    }

    fn on_key_log(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => self.log_scroll = self.log_scroll.saturating_add(1),
            KeyCode::Down | KeyCode::Char('j') => self.log_scroll = self.log_scroll.saturating_sub(1),
            KeyCode::PageUp => self.log_scroll = self.log_scroll.saturating_add(10),
            KeyCode::PageDown => self.log_scroll = self.log_scroll.saturating_sub(10),
            _ => {}
        }
    }
}
