use std::net::SocketAddr;
use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyEvent};
use nicer::event::{Command, Verdict};
use nicer_proto::payload::OfferKind;

use super::{App, ChatLine, Modal, RequestKind};
use crate::theme;

enum TextOutcome {
    Continue,
    Submit,
    Cancel,
}

impl App {
    fn text_key(&mut self, key: &KeyEvent) -> TextOutcome {
        match key.code {
            KeyCode::Esc => TextOutcome::Cancel,
            KeyCode::Enter => TextOutcome::Submit,
            KeyCode::Backspace => {
                self.input.pop();
                TextOutcome::Continue
            }
            KeyCode::Char(c) => {
                self.input.push(c);
                TextOutcome::Continue
            }
            _ => TextOutcome::Continue,
        }
    }

    pub(super) fn on_modal_key(&mut self, key: KeyEvent) {
        match &self.modal {
            Modal::Help => self.close_modal(),
            Modal::Pairing(prompt) => {
                let prompt = prompt.clone();
                match key.code {
                    KeyCode::Char('y') | KeyCode::Enter => {
                        self.dispatch(
                            RequestKind::Pair,
                            Command::Pair {
                                fingerprint: prompt.fingerprint,
                                device: Some(prompt.device.clone()),
                            },
                        );
                        self.close_modal();
                        self.maybe_pop_pairing_popup();
                    }
                    KeyCode::Char('n') | KeyCode::Esc => {
                        self.close_modal();
                        self.maybe_pop_pairing_popup();
                    }
                    _ => {}
                }
            }
            Modal::Offer { connection, stream } => {
                let (connection, stream) = (*connection, *stream);
                match key.code {
                    KeyCode::Char('m') => {
                        self.dispatch(
                            RequestKind::Respond,
                            Command::Respond { connection, stream, verdict: Verdict::Merge },
                        );
                        self.close_modal();
                        self.maybe_pop_pairing_popup();
                    }
                    KeyCode::Char('f') => {
                        self.dispatch(
                            RequestKind::Respond,
                            Command::Respond {
                                connection,
                                stream,
                                verdict: Verdict::FuckOff { reason: None },
                            },
                        );
                        self.close_modal();
                        self.maybe_pop_pairing_popup();
                    }
                    KeyCode::Char('F') => self.modal = Modal::FuckOffReason { connection, stream },
                    KeyCode::Char('b') => self.modal = Modal::BigDiffSize { connection, stream },
                    KeyCode::Esc => {
                        self.close_modal();
                        self.maybe_pop_pairing_popup();
                    }
                    _ => {}
                }
            }
            Modal::ConfirmUnpair { fingerprint, .. } => {
                let fingerprint = *fingerprint;
                match key.code {
                    KeyCode::Char('y') | KeyCode::Enter => {
                        self.dispatch(RequestKind::Unpair, Command::Unpair { fingerprint });
                        self.close_modal();
                    }
                    KeyCode::Char('n') | KeyCode::Esc => self.close_modal(),
                    _ => {}
                }
            }
            Modal::Connect => match self.text_key(&key) {
                TextOutcome::Submit => {
                    let text = self.input.trim().to_string();
                    match text.parse::<SocketAddr>() {
                        Ok(address) => {
                            self.dispatch(RequestKind::Connect, Command::Connect { address });
                            self.close_modal();
                        }
                        Err(_) => self.set_status("expected host:port", theme::CUT),
                    }
                }
                TextOutcome::Cancel => self.close_modal(),
                TextOutcome::Continue => {}
            },
            Modal::SendFile { connection } => {
                let connection = *connection;
                match self.text_key(&key) {
                    TextOutcome::Submit => {
                        let path = PathBuf::from(self.input.trim());
                        let name = path.file_name().map(|n| n.to_string_lossy().into_owned());
                        self.pending_sends.entry(connection).or_default().push_back((OfferKind::File, name));
                        self.dispatch(RequestKind::SendFile, Command::SendFile { connection, path });
                        self.close_modal();
                    }
                    TextOutcome::Cancel => self.close_modal(),
                    TextOutcome::Continue => {}
                }
            }
            Modal::SendClipboardText { connection } => {
                let connection = *connection;
                match self.text_key(&key) {
                    TextOutcome::Submit => {
                        let text = self.input.clone();
                        self.pending_sends
                            .entry(connection)
                            .or_default()
                            .push_back((OfferKind::Clipboard, None));
                        self.dispatch(
                            RequestKind::SendClipboard,
                            Command::SendClipboard { connection, text: Some(text) },
                        );
                        self.close_modal();
                    }
                    TextOutcome::Cancel => self.close_modal(),
                    TextOutcome::Continue => {}
                }
            }
            Modal::ChatInput { connection } => {
                let connection = *connection;
                match self.text_key(&key) {
                    TextOutcome::Submit => {
                        let text = self.input.clone();
                        if !text.is_empty() {
                            self.chats.entry(connection).or_default().push(ChatLine {
                                from_us: true,
                                from: "you".into(),
                                fingerprint: self.fingerprint,
                                text: text.clone(),
                            });
                            self.dispatch(RequestKind::Chat, Command::Chat { connection, text });
                        }
                        self.close_modal();
                    }
                    TextOutcome::Cancel => self.close_modal(),
                    TextOutcome::Continue => {}
                }
            }
            Modal::BigDiffSize { connection, stream } => {
                let (connection, stream) = (*connection, *stream);
                match self.text_key(&key) {
                    TextOutcome::Submit => {
                        let max_size = self.input.trim().parse::<u64>().ok();
                        self.dispatch(
                            RequestKind::Respond,
                            Command::Respond { connection, stream, verdict: Verdict::BigDiff { max_size } },
                        );
                        self.close_modal();
                        self.maybe_pop_pairing_popup();
                    }
                    TextOutcome::Cancel => self.close_modal(),
                    TextOutcome::Continue => {}
                }
            }
            Modal::FuckOffReason { connection, stream } => {
                let (connection, stream) = (*connection, *stream);
                match self.text_key(&key) {
                    TextOutcome::Submit => {
                        let reason = if self.input.is_empty() { None } else { Some(self.input.clone()) };
                        self.dispatch(
                            RequestKind::Respond,
                            Command::Respond { connection, stream, verdict: Verdict::FuckOff { reason } },
                        );
                        self.close_modal();
                        self.maybe_pop_pairing_popup();
                    }
                    TextOutcome::Cancel => self.close_modal(),
                    TextOutcome::Continue => {}
                }
            }
            Modal::None => {}
        }
    }
}
