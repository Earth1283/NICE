use nicer::event::{ConnectionSummary, Event};

use super::{offer_kind_label, App, ChatLine, Discovered, Modal, PairingPrompt, TransferStatus};
use crate::theme;

impl App {
    pub fn on_event(&mut self, event: Event) {
        self.push_tick(&event);

        match event {
            Event::Listening {
                address,
                device,
                fingerprint,
                ..
            } => {
                self.listen = Some(address);
                self.device = device.clone();
                self.fingerprint = Some(fingerprint);
                self.push_log(format!("listening on {address} as {device}"), theme::NICE);
            }
            Event::PeerDiscovered {
                instance,
                device,
                addresses,
                port,
                secure,
                fingerprint,
            } => {
                self.push_log(format!("discovered {device} ({instance})"), theme::SIGNAL);
                self.discovered.insert(
                    instance,
                    Discovered {
                        device,
                        addresses,
                        port,
                        secure,
                        fingerprint,
                    },
                );
            }
            Event::PeerLost { instance } => {
                self.discovered.remove(&instance);
                self.push_log(format!("lost {instance}"), theme::SLATE);
            }
            Event::Connected {
                connection,
                address,
                device,
                transport,
                direction,
                fingerprint,
                paired,
            } => {
                self.push_log(
                    format!("{connection} {direction:?} {device} ({address}) {transport:?} paired={paired}"),
                    theme::NICE,
                );
                self.connections.insert(
                    connection,
                    ConnectionSummary {
                        connection,
                        address,
                        device,
                        transport,
                        direction,
                        fingerprint,
                        paired,
                    },
                );
            }
            Event::Disconnected { connection, reason } => {
                self.push_log(format!("{connection} disconnected: {reason}"), theme::SLATE);
                self.connections.remove(&connection);
                self.pending_pairing_popups.retain(|c| *c != connection);
                self.pending_offer_popups.retain(|(c, _)| *c != connection);
            }
            Event::PairingRequired {
                connection,
                address,
                device,
                fingerprint,
                short,
            } => {
                self.push_log(format!("{connection} needs pairing: {device} {short}"), theme::WIRE);
                self.pending_pairing_popups.push_back(connection);
                if matches!(self.modal, Modal::None) {
                    self.modal = Modal::Pairing(PairingPrompt {
                        address,
                        device,
                        fingerprint,
                        short,
                    });
                    self.pending_pairing_popups.pop_front();
                }
            }
            Event::IdentityChanged {
                address,
                expected,
                reason,
            } => {
                self.push_log(
                    format!("IDENTITY CHANGED for {address}: expected {} — {reason}", expected.short()),
                    theme::CUT,
                );
                self.set_status(format!("identity changed for {address}: {reason}"), theme::CUT);
            }
            Event::Offer {
                connection,
                stream,
                kind,
                size,
                name,
                mime,
                preview,
                auto_accepted,
            } => {
                self.push_log(
                    format!(
                        "{connection}/{stream} offers {} {} ({size}B) auto_accepted={auto_accepted}",
                        offer_kind_label(kind),
                        name.as_deref().unwrap_or("-"),
                    ),
                    theme::SIGNAL,
                );
                self.upsert_transfer(
                    connection,
                    stream,
                    name,
                    TransferStatus::Offered { kind, size, mime, preview },
                );
                if !auto_accepted {
                    self.pending_offer_popups.push_back((connection, stream));
                    if matches!(self.modal, Modal::None) {
                        self.modal = Modal::Offer { connection, stream };
                        self.pending_offer_popups.pop_front();
                    }
                }
            }
            Event::OfferResolved {
                connection,
                stream,
                verdict,
                ..
            } => {
                self.push_log(format!("{connection}/{stream} resolved: {verdict:?}"), theme::WIRE);
                self.resolve_offer(connection, stream, verdict);
            }
            Event::TransferProgress {
                connection,
                stream,
                direction,
                transferred,
                total,
            } => {
                let kind = self.known_or_pending(connection, stream).0.unwrap_or(nicer_proto::payload::OfferKind::File);
                self.upsert_transfer(
                    connection,
                    stream,
                    None,
                    TransferStatus::Active { direction, kind, transferred, total },
                );
            }
            Event::TransferComplete {
                connection,
                stream,
                direction,
                kind,
                path,
            } => {
                self.push_log(
                    format!(
                        "{connection}/{stream} complete: {}{}",
                        offer_kind_label(kind),
                        path.as_ref().map(|p| format!(" -> {}", p.display())).unwrap_or_default()
                    ),
                    theme::NICE,
                );
                self.upsert_transfer(connection, stream, None, TransferStatus::Complete { direction, kind, path });
            }
            Event::TransferFailed {
                connection,
                stream,
                direction,
                reason,
            } => {
                self.push_log(format!("{connection}/{stream} failed: {reason}"), theme::CUT);
                self.upsert_transfer(connection, stream, None, TransferStatus::Failed { direction, reason });
            }
            Event::Chat {
                connection,
                from,
                fingerprint,
                text,
                ..
            } => {
                self.push_log(format!("{connection} <{from}> {text}"), theme::BONE);
                self.chats.entry(connection).or_default().push(ChatLine {
                    from_us: false,
                    from: from.to_string(),
                    fingerprint,
                    text,
                });
                if self.chat_connection.is_none() {
                    self.chat_connection = Some(connection);
                }
            }
            Event::RateLimited {
                connection,
                direction,
                retry_after_ms,
                scope,
            } => {
                self.push_log(
                    format!("{connection} rate limited ({direction:?}, {scope:?}, {retry_after_ms}ms)"),
                    theme::WIRE,
                );
            }
            Event::ProtocolError { connection, opcode, reason } => {
                self.push_log(format!("{connection} {opcode}: {reason}"), theme::CUT);
                self.set_status(format!("{connection} {opcode}: {reason}"), theme::CUT);
            }
            Event::Resynced { connection } => {
                self.push_log(format!("{connection} resynced (BITKEEPER/GIT)"), theme::WIRE);
                self.pending_offer_popups.retain(|(c, _)| *c != connection);
            }
            Event::Warning { message } => {
                self.push_log(format!("warning: {message}"), theme::WIRE);
                self.set_status(message, theme::WIRE);
            }
        }
    }
}
