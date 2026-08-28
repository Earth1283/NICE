use std::collections::{HashMap, VecDeque};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use nicer_proto::payload::{
    self, Hello, Lkml, Merged, OfferKind, PullRequest, Reason, ShutUpScope, TransportMode,
};
use nicer_proto::{
    Diff, Frame, Incoming, NiceCodec, Opcode, PeerStreams, ProtoError, Role, Scope,
    StreamAllocator, StreamId, PROTOCOL_VERSION,
};
use tokio::sync::mpsc;
use tokio_util::codec::Framed;

use crate::clipboard::Clipboard;
use crate::config::{sanitize_display, Config};
use crate::error::{Error, Result};
use crate::event::{ConnectionId, Direction, Event, Verdict};
use crate::identity::{Fingerprint, Identity};
use crate::pairing::{now_millis, PairingStore};
use crate::ratelimit::{shut_up, ConnectionLimiter, Restraint};
use crate::transfer::{Algorithm, Received, Receiving, Sending};
use crate::transport::Stream;

const PROGRESS_INTERVAL: u64 = 1024 * 1024;
const OFFER_PATIENCE: Duration = Duration::from_secs(300);
const SHUT_UP_COOLDOWN: Duration = Duration::from_secs(5);
const MAX_SHUT_UPS: u32 = 5;

pub struct Shared {
    pub config: Config,
    pub identity: Arc<Identity>,
    pub pairing: Arc<Mutex<PairingStore>>,
    pub clipboard: Arc<dyn Clipboard>,
}

#[derive(Clone, Debug)]
pub struct PeerInfo {
    pub device: String,
    pub fingerprint: String,
    pub transport: TransportMode,
    pub max_frame_size: u32,
    pub capabilities: Vec<String>,
}

pub enum SessionCommand {
    OfferFile(PathBuf),
    OfferClipboard(Option<String>),
    Respond { stream: StreamId, verdict: Verdict },
    Chat(String),
    Resync,
    Close(String),
}

#[derive(Clone, Debug)]
pub struct SessionInfo {
    pub address: SocketAddr,
    pub device: String,
    pub fingerprint: Option<Fingerprint>,
    pub paired: bool,
    pub transport: TransportMode,
    pub direction: Direction,
}

/// How a connection ends. Every variant but `Closed` puts an error opcode on the wire first.
enum Fatal {
    BrokeUserspace(String),
    Nvidia(String),
    Closed(String),
    Transport(Error),
}

impl From<Error> for Fatal {
    fn from(e: Error) -> Self {
        Fatal::Transport(e)
    }
}

impl From<ProtoError> for Fatal {
    fn from(e: ProtoError) -> Self {
        Fatal::Transport(Error::Proto(e))
    }
}

type Step = std::result::Result<(), Fatal>;

struct ActiveSend {
    sending: Sending,
    kind: OfferKind,
    reported: u64,
}

enum StreamState {
    OfferedByUs {
        offer: PullRequest,
        source: PendingSource,
    },
    SendingToPeer,
    AwaitingVerification {
        kind: OfferKind,
    },
    OfferedToUs {
        offer: PullRequest,
        since: Instant,
    },
    ReceivingFromPeer {
        offer: PullRequest,
        receiving: Receiving,
        reported: u64,
    },
    ReceivedAwaitingFsck {
        offer: PullRequest,
        receiving: Receiving,
    },
    Chat,
}

enum PendingSource {
    File(PathBuf),
    Memory(Bytes),
}

pub struct Session {
    id: ConnectionId,
    framed: Framed<Stream, NiceCodec>,
    shared: Arc<Shared>,
    role: Role,
    address: SocketAddr,
    peer: PeerInfo,
    fingerprint: Option<Fingerprint>,
    paired: bool,
    allocator: StreamAllocator,
    peer_streams: PeerStreams,
    streams: HashMap<StreamId, StreamState>,
    sends: HashMap<StreamId, ActiveSend>,
    send_queue: VecDeque<StreamId>,
    chat_send: Option<StreamId>,
    peer_chat: Option<StreamId>,
    limiter: ConnectionLimiter,
    restraint: Restraint,
    events: mpsc::Sender<Event>,
    commands: mpsc::Receiver<SessionCommand>,
    last_activity: Instant,
    keepalives_outstanding: u32,
    last_shut_up: Option<Instant>,
    partial_since: Option<Instant>,
    shut_ups_sent: u32,
    message_counter: u64,
}

impl Session {
    /// Performs `HELLO`/`MERGED` and returns a session ready to run (`NICE-1.md` §8).
    pub async fn establish(
        id: ConnectionId,
        stream: Stream,
        address: SocketAddr,
        role: Role,
        shared: Arc<Shared>,
        events: mpsc::Sender<Event>,
        commands: mpsc::Receiver<SessionCommand>,
    ) -> Result<Self> {
        let fingerprint = stream.peer_fingerprint()?;
        let transport = stream.mode();
        let mut framed = Framed::new(stream, NiceCodec::new(shared.config.max_frame_size));

        let greeting = Hello {
            version: PROTOCOL_VERSION,
            device: shared.config.device_name.clone(),
            fingerprint: shared.identity.fingerprint().to_string(),
            transport,
            max_frame_size: shared.config.max_frame_size,
            capabilities: capabilities(),
        };

        let (version, peer) = match role {
            Role::Initiator => {
                framed
                    .send(Frame::control_cbor(Opcode::Hello, &greeting)?)
                    .await?;
                let merged: Merged = expect(&mut framed, Opcode::Merged).await?.parse()?;
                (
                    merged.version,
                    PeerInfo {
                        device: sanitize_display(&merged.device, payload::MAX_DEVICE_NAME),
                        fingerprint: merged.fingerprint,
                        transport: merged.transport,
                        max_frame_size: merged.max_frame_size,
                        capabilities: merged.capabilities,
                    },
                )
            }
            Role::Responder => {
                let hello: Hello = expect(&mut framed, Opcode::Hello).await?.parse()?;
                let answer = Merged {
                    version: greeting.version,
                    device: greeting.device.clone(),
                    fingerprint: greeting.fingerprint.clone(),
                    transport: greeting.transport,
                    max_frame_size: greeting.max_frame_size,
                    capabilities: greeting.capabilities.clone(),
                };
                framed
                    .send(Frame::control_cbor(Opcode::Merged, &answer)?)
                    .await?;
                (
                    hello.version,
                    PeerInfo {
                        device: sanitize_display(&hello.device, payload::MAX_DEVICE_NAME),
                        fingerprint: hello.fingerprint,
                        transport: hello.transport,
                        max_frame_size: hello.max_frame_size,
                        capabilities: hello.capabilities,
                    },
                )
            }
        };

        if version != PROTOCOL_VERSION {
            return Err(Error::Rejected(format!("peer speaks NICE/{version}")));
        }
        if peer.max_frame_size < nicer_proto::MIN_MAX_FRAME_SIZE {
            return Err(Error::Rejected(format!(
                "peer advertised max_frame_size {}, below the mandatory floor of {}",
                peer.max_frame_size,
                nicer_proto::MIN_MAX_FRAME_SIZE
            )));
        }
        if let Some(authenticated) = fingerprint {
            if peer.fingerprint != authenticated.to_string() {
                return Err(Error::Rejected(format!(
                    "the greeting claims {} but the certificate proves {}",
                    peer.fingerprint,
                    authenticated.short()
                )));
            }
        }

        framed.codec_mut().set_peer_limit(peer.max_frame_size);

        let paired = match fingerprint {
            Some(fp) => {
                let mut store = shared.pairing.lock().expect("pairing store");
                store.observe(&fp, address.ip(), &peer.device);
                store.is_paired(&fp)
            }
            None => false,
        };

        Ok(Self {
            id,
            framed,
            limiter: ConnectionLimiter::new(&shared.config.limits),
            allocator: StreamAllocator::new(role),
            peer_streams: PeerStreams::new(role),
            shared,
            role,
            address,
            peer,
            fingerprint,
            paired,
            streams: HashMap::new(),
            sends: HashMap::new(),
            send_queue: VecDeque::new(),
            chat_send: None,
            peer_chat: None,
            restraint: Restraint::default(),
            events,
            commands,
            last_activity: Instant::now(),
            keepalives_outstanding: 0,
            last_shut_up: None,
            partial_since: None,
            shut_ups_sent: 0,
            message_counter: 0,
        })
    }

    pub fn id(&self) -> ConnectionId {
        self.id
    }

    pub fn info(&self) -> SessionInfo {
        SessionInfo {
            address: self.address,
            device: self.peer.device.clone(),
            fingerprint: self.fingerprint,
            paired: self.paired,
            transport: self.peer.transport,
            direction: match self.role {
                Role::Initiator => Direction::Outgoing,
                Role::Responder => Direction::Incoming,
            },
        }
    }

    pub async fn run(mut self) -> String {
        let reason = match self.serve().await {
            Ok(reason) => reason,
            Err(Fatal::BrokeUserspace(reason)) => {
                self.report(Opcode::BrokeUserspace, &reason).await;
                reason
            }
            Err(Fatal::Nvidia(reason)) => {
                self.report(Opcode::Nvidia, &reason).await;
                reason
            }
            Err(Fatal::Closed(reason)) => reason,
            Err(Fatal::Transport(e)) => e.to_string(),
        };
        self.abandon_streams().await;
        let _ = self.framed.close().await;
        reason
    }

    async fn serve(&mut self) -> std::result::Result<String, Fatal> {
        let mut ticker = tokio::time::interval(Duration::from_secs(1));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        loop {
            let sending = !self.send_queue.is_empty();

            let wake = tokio::select! {
                biased;
                command = self.commands.recv() => Wake::Command(command),
                frame = self.framed.next() => Wake::Frame(frame),
                _ = ticker.tick() => Wake::Tick,
                _ = std::future::ready(()), if sending => Wake::Send,
            };

            match wake {
                Wake::Command(None) => return Ok("the node dropped this connection".into()),
                Wake::Command(Some(SessionCommand::Close(reason))) => return Ok(reason),
                Wake::Command(Some(command)) => self.on_command(command).await?,
                Wake::Frame(None) => return Ok("peer closed the connection".into()),
                Wake::Frame(Some(Ok(incoming))) => {
                    self.last_activity = Instant::now();
                    self.keepalives_outstanding = 0;
                    self.partial_since = None;
                    self.throttle().await?;
                    self.on_incoming(incoming).await?;
                }
                Wake::Frame(Some(Err(e))) => {
                    if !e.is_framing_loss() {
                        self.report(Opcode::Cpp, &e.to_string()).await;
                    }
                    return Err(Fatal::Closed(e.to_string()));
                }
                Wake::Tick => self.on_tick().await?,
                Wake::Send => self.push_one_chunk().await?,
            }
        }
    }

    async fn on_command(&mut self, command: SessionCommand) -> Step {
        match command {
            SessionCommand::OfferFile(path) => self.offer_file(path).await,
            SessionCommand::OfferClipboard(text) => self.offer_clipboard(text).await,
            SessionCommand::Respond { stream, verdict } => self.answer_offer(stream, verdict).await,
            SessionCommand::Chat(text) => self.say(text).await,
            SessionCommand::Resync => {
                self.reset().await;
                self.send(Frame::control(Opcode::Bitkeeper)).await
            }
            SessionCommand::Close(reason) => Err(Fatal::Closed(reason)),
        }
    }

    async fn offer_file(&mut self, path: PathBuf) -> Step {
        if self.held_back(Opcode::PullRequest).await {
            return Ok(());
        }
        let metadata = match tokio::fs::metadata(&path).await {
            Ok(metadata) => metadata,
            Err(e) => return self.warn(format!("{}: {e}", path.display())).await,
        };
        if !metadata.is_file() {
            return self
                .warn(format!("{} is not a regular file", path.display()))
                .await;
        }
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "received".into());
        let offer = PullRequest {
            kind: OfferKind::File,
            size: metadata.len(),
            mime: None,
            name: Some(name),
            preview: None,
        };
        self.open_offer(offer, PendingSource::File(path)).await
    }

    async fn offer_clipboard(&mut self, text: Option<String>) -> Step {
        if self.held_back(Opcode::PullRequest).await {
            return Ok(());
        }
        let text = match text {
            Some(text) => text,
            None => {
                let clipboard = self.shared.clipboard.clone();
                match tokio::task::spawn_blocking(move || clipboard.read_text()).await {
                    Ok(Ok(text)) => text,
                    Ok(Err(e)) => return self.warn(e.to_string()).await,
                    Err(e) => return self.warn(e.to_string()).await,
                }
            }
        };
        if text.is_empty() {
            return self.warn("the clipboard is empty").await;
        }
        let bytes = Bytes::from(text.clone().into_bytes());
        let offer = PullRequest {
            kind: OfferKind::Clipboard,
            size: bytes.len() as u64,
            mime: Some("text/plain".into()),
            name: None,
            preview: Some(preview_of(&text)),
        };
        self.open_offer(offer, PendingSource::Memory(bytes)).await
    }

    async fn open_offer(&mut self, offer: PullRequest, source: PendingSource) -> Step {
        if self.streams.len() >= self.shared.config.limits.max_streams_per_connection {
            return self
                .warn("too many streams are already open on this connection")
                .await;
        }
        let Some(stream) = self.allocator.allocate() else {
            return Err(Fatal::Closed(
                "stream identifiers are exhausted; reconnect to continue".into(),
            ));
        };
        self.send(Frame::cbor(Opcode::PullRequest, stream, &offer)?)
            .await?;
        self.streams
            .insert(stream, StreamState::OfferedByUs { offer, source });
        Ok(())
    }

    async fn say(&mut self, text: String) -> Step {
        if text.len() > payload::MAX_CHAT_TEXT {
            return self
                .warn(format!(
                    "a chat message may carry {} octets, not {}",
                    payload::MAX_CHAT_TEXT,
                    text.len()
                ))
                .await;
        }
        if self.held_back(Opcode::Lkml).await {
            return Ok(());
        }
        let stream = match self.conversation() {
            Some(stream) => stream,
            None => {
                let Some(stream) = self.allocator.allocate() else {
                    return Err(Fatal::Closed("stream identifiers are exhausted".into()));
                };
                self.streams.insert(stream, StreamState::Chat);
                self.chat_send = Some(stream);
                stream
            }
        };
        self.message_counter += 1;
        let message = Lkml {
            message_id: format!("{}-{}", now_millis(), self.message_counter),
            timestamp: now_millis(),
            text,
            tags: None,
        };
        self.send(Frame::cbor(Opcode::Lkml, stream, &message)?)
            .await
    }

    /// RFC R10: one conversation per connection, and the initiator's stream wins a tie.
    fn conversation(&mut self) -> Option<StreamId> {
        if let Some(stream) = self.chat_send {
            return Some(stream);
        }
        if let Some(stream) = self.peer_chat {
            if self.role == Role::Responder {
                self.chat_send = Some(stream);
                return Some(stream);
            }
        }
        None
    }

    async fn answer_offer(&mut self, stream: StreamId, verdict: Verdict) -> Step {
        let Some(StreamState::OfferedToUs { offer, .. }) = self.streams.get(&stream) else {
            return self
                .warn(format!("stream {stream} is not waiting for a verdict"))
                .await;
        };
        let offer = offer.clone();

        match &verdict {
            Verdict::Merge => {
                let receiving = match offer.kind {
                    OfferKind::File => {
                        Receiving::into_file(
                            &self.shared.config.download_dir,
                            offer.name.as_deref().unwrap_or("received"),
                            offer.size,
                        )
                        .await
                    }
                    OfferKind::Clipboard => Ok(Receiving::into_memory(offer.size)),
                };
                match receiving {
                    Ok(receiving) => {
                        self.send(Frame::bare(Opcode::Merge, stream)).await?;
                        self.streams.insert(
                            stream,
                            StreamState::ReceivingFromPeer {
                                offer,
                                receiving,
                                reported: 0,
                            },
                        );
                    }
                    Err(e) => {
                        self.streams.remove(&stream);
                        let refusal = payload::FuckOff {
                            reason: Some("the receiver could not open a destination".into()),
                        };
                        self.send(Frame::cbor(Opcode::FuckOff, stream, &refusal)?)
                            .await?;
                        self.emit(Event::TransferFailed {
                            connection: self.id,
                            stream: stream.0,
                            direction: Direction::Incoming,
                            reason: e.to_string(),
                        })
                        .await;
                        return Ok(());
                    }
                }
            }
            Verdict::FuckOff { reason } => {
                self.streams.remove(&stream);
                let refusal = payload::FuckOff {
                    reason: reason.clone(),
                };
                self.send(Frame::cbor(Opcode::FuckOff, stream, &refusal)?)
                    .await?;
            }
            Verdict::BigDiff { max_size } => {
                self.streams.remove(&stream);
                let refusal = payload::BigDiff {
                    max_size: max_size.or(Some(self.shared.config.limits.max_object_size)),
                };
                self.send(Frame::cbor(Opcode::BigDiff, stream, &refusal)?)
                    .await?;
            }
        }

        self.emit(Event::OfferResolved {
            connection: self.id,
            stream: stream.0,
            direction: Direction::Outgoing,
            verdict,
        })
        .await;
        Ok(())
    }

    async fn on_incoming(&mut self, incoming: Incoming) -> Step {
        let frame = match incoming {
            Incoming::Frame(frame) => frame,
            Incoming::Unknown { opcode, stream, .. } => {
                return self
                    .cpp(
                        stream,
                        format!("opcode 0x{opcode:02x} is not defined in NICE/1"),
                    )
                    .await;
            }
        };

        match frame.opcode.scope() {
            Scope::Connection if !frame.stream.is_connection() => {
                return Err(Fatal::BrokeUserspace(format!(
                    "{} belongs on stream 0, not {}",
                    frame.opcode, frame.stream
                )));
            }
            Scope::Stream if frame.stream.is_connection() => {
                return Err(Fatal::BrokeUserspace(format!(
                    "{} must not use stream 0",
                    frame.opcode
                )));
            }
            _ => {}
        }

        match frame.opcode {
            Opcode::Hello | Opcode::Merged => Err(Fatal::BrokeUserspace(format!(
                "{} arrived after the handshake was already complete",
                frame.opcode
            ))),
            Opcode::Tux => self.send(Frame::control(Opcode::Subsurface)).await,
            Opcode::Subsurface => Ok(()),
            Opcode::PullRequest => self.on_offer(frame).await,
            Opcode::Merge => self.on_merge(frame).await,
            Opcode::FuckOff | Opcode::BigDiff => self.on_refusal(frame).await,
            Opcode::ShutUp => self.on_shut_up(frame).await,
            Opcode::Diff => self.on_diff(frame).await,
            Opcode::Done => self.on_done(frame).await,
            Opcode::Fsck => self.on_fsck(frame).await,
            Opcode::Clean | Opcode::Corrupt => self.on_verification(frame).await,
            Opcode::Lkml => self.on_chat(frame).await,
            Opcode::Bitkeeper => {
                self.reset().await;
                self.send(Frame::control(Opcode::Git)).await?;
                self.emit(Event::Resynced {
                    connection: self.id,
                })
                .await;
                Ok(())
            }
            Opcode::Git => {
                self.reset().await;
                self.emit(Event::Resynced {
                    connection: self.id,
                })
                .await;
                Ok(())
            }
            Opcode::Monotone => Err(Fatal::Closed(format!(
                "peer declined to resynchronise: {}",
                frame.parse_or_default::<Reason>().display()
            ))),
            Opcode::Cpp => {
                let reason = frame.parse_or_default::<Reason>();
                self.emit(Event::ProtocolError {
                    connection: self.id,
                    opcode: Opcode::Cpp.name().into(),
                    reason: reason.display().to_string(),
                })
                .await;
                Ok(())
            }
            Opcode::BrokeUserspace | Opcode::Nvidia => Err(Fatal::Closed(format!(
                "peer sent {}: {}",
                frame.opcode,
                frame.parse_or_default::<Reason>().display()
            ))),
        }
    }

    async fn on_offer(&mut self, frame: Frame) -> Step {
        let offer: PullRequest = match frame.parse() {
            Ok(offer) => offer,
            Err(e) => return self.cpp(frame.stream, e.to_string()).await,
        };
        if let Err(e) = offer.validate() {
            return self.cpp(frame.stream, e.to_string()).await;
        }
        self.peer_streams
            .accept_new(frame.stream)
            .map_err(|v| Fatal::BrokeUserspace(v.to_string()))?;

        let limits = &self.shared.config.limits;
        let too_many = self.streams.len() >= limits.max_streams_per_connection
            || self.pending_offers() >= limits.max_pending_offers;
        if too_many {
            return self
                .rate_limit(frame.stream, Duration::from_secs(5), Opcode::PullRequest)
                .await;
        }
        if let Err(delay) = self.limiter.offers.take() {
            return self
                .rate_limit(frame.stream, delay, Opcode::PullRequest)
                .await;
        }
        if offer.size > limits.max_object_size {
            let refusal = payload::BigDiff {
                max_size: Some(limits.max_object_size),
            };
            self.send(Frame::cbor(Opcode::BigDiff, frame.stream, &refusal)?)
                .await?;
            self.emit(Event::OfferResolved {
                connection: self.id,
                stream: frame.stream.0,
                direction: Direction::Outgoing,
                verdict: Verdict::BigDiff {
                    max_size: refusal.max_size,
                },
            })
            .await;
            return Ok(());
        }

        let auto_accepted = self.auto_accepts(offer.kind);
        self.emit(Event::Offer {
            connection: self.id,
            stream: frame.stream.0,
            kind: offer.kind,
            size: offer.size,
            name: offer
                .name
                .as_deref()
                .map(crate::transfer::sanitize_filename),
            mime: offer.mime.clone(),
            preview: offer
                .preview
                .as_deref()
                .map(|p| sanitize_display(p, payload::MAX_PREVIEW)),
            auto_accepted,
        })
        .await;

        self.streams.insert(
            frame.stream,
            StreamState::OfferedToUs {
                offer,
                since: Instant::now(),
            },
        );

        if auto_accepted {
            self.answer_offer(frame.stream, Verdict::Merge).await?;
        }
        Ok(())
    }

    async fn on_merge(&mut self, frame: Frame) -> Step {
        let Some(StreamState::OfferedByUs { offer, source }) = self.streams.remove(&frame.stream)
        else {
            return Err(Fatal::BrokeUserspace(format!(
                "MERGE names stream {}, which carries no offer of ours",
                frame.stream
            )));
        };

        let sending = match source {
            PendingSource::File(path) => Sending::from_path(&path).await.map(|(s, _)| s),
            PendingSource::Memory(bytes) => Ok(Sending::from_bytes(bytes)),
        };
        let sending = match sending {
            Ok(sending) if sending.size() == offer.size => sending,
            Ok(_) => {
                return self
                    .withdraw(frame.stream, "the object changed after it was offered")
                    .await
            }
            Err(e) => return self.withdraw(frame.stream, e.to_string()).await,
        };

        self.sends.insert(
            frame.stream,
            ActiveSend {
                sending,
                kind: offer.kind,
                reported: 0,
            },
        );
        self.send_queue.push_back(frame.stream);
        self.streams
            .insert(frame.stream, StreamState::SendingToPeer);
        self.emit(Event::OfferResolved {
            connection: self.id,
            stream: frame.stream.0,
            direction: Direction::Incoming,
            verdict: Verdict::Merge,
        })
        .await;
        Ok(())
    }

    async fn push_one_chunk(&mut self) -> Step {
        let Some(stream) = self.send_queue.pop_front() else {
            return Ok(());
        };
        let budget = self
            .shared
            .config
            .chunk_size_for(self.framed.codec().peer_limit());

        let chunk = match self.sends.get_mut(&stream) {
            Some(active) => active.sending.next_chunk(budget).await,
            None => return Ok(()),
        };

        match chunk {
            Ok(Some((offset, data))) => {
                self.send(Diff { offset, data }.into_frame(stream)).await?;
                let progress = self.sends.get_mut(&stream).map(|active| {
                    let transferred = active.sending.offset();
                    let due = transferred - active.reported >= PROGRESS_INTERVAL;
                    if due {
                        active.reported = transferred;
                    }
                    (due, transferred, active.sending.size())
                });
                if let Some((true, transferred, total)) = progress {
                    self.emit(Event::TransferProgress {
                        connection: self.id,
                        stream: stream.0,
                        direction: Direction::Outgoing,
                        transferred,
                        total,
                    })
                    .await;
                }
                self.send_queue.push_back(stream);
                Ok(())
            }
            Ok(None) => {
                let Some(active) = self.sends.remove(&stream) else {
                    return Ok(());
                };
                let fsck = payload::Fsck {
                    algorithm: payload::ALGORITHM_BLAKE3.into(),
                    digest: active.sending.digest(),
                    total_size: active.sending.size(),
                };
                self.send(Frame::bare(Opcode::Done, stream)).await?;
                self.send(Frame::cbor(Opcode::Fsck, stream, &fsck)?).await?;
                self.streams.insert(
                    stream,
                    StreamState::AwaitingVerification { kind: active.kind },
                );
                Ok(())
            }
            Err(e) => {
                self.sends.remove(&stream);
                self.withdraw(stream, e.to_string()).await
            }
        }
    }

    async fn on_diff(&mut self, frame: Frame) -> Step {
        let diff = match Diff::decode(frame.payload.clone()) {
            Ok(diff) => diff,
            Err(e) => return self.cpp(frame.stream, e.to_string()).await,
        };

        let outcome = match self.streams.get_mut(&frame.stream) {
            Some(StreamState::ReceivingFromPeer {
                receiving,
                reported,
                ..
            }) => match receiving.write(diff.offset, &diff.data).await {
                Ok(()) => {
                    let transferred = receiving.written();
                    let due = transferred - *reported >= PROGRESS_INTERVAL;
                    if due {
                        *reported = transferred;
                    }
                    Ok((due, transferred, receiving.expected()))
                }
                Err(e) => Err(e.to_string()),
            },
            Some(StreamState::OfferedToUs { .. }) => Err(format!(
                "DIFF on stream {} before MERGE was sent",
                frame.stream
            )),
            _ => Err(format!(
                "DIFF on stream {}, which carries no accepted offer",
                frame.stream
            )),
        };

        match outcome {
            Ok((true, transferred, total)) => {
                self.emit(Event::TransferProgress {
                    connection: self.id,
                    stream: frame.stream.0,
                    direction: Direction::Incoming,
                    transferred,
                    total,
                })
                .await;
                Ok(())
            }
            Ok(_) => Ok(()),
            Err(reason) => Err(Fatal::BrokeUserspace(reason)),
        }
    }

    async fn on_done(&mut self, frame: Frame) -> Step {
        let Some(StreamState::ReceivingFromPeer {
            offer, receiving, ..
        }) = self.streams.remove(&frame.stream)
        else {
            return Err(Fatal::BrokeUserspace(format!(
                "DONE on stream {}, which is not receiving an object",
                frame.stream
            )));
        };
        if !receiving.is_complete() {
            let written = receiving.written();
            receiving.discard().await;
            return Err(Fatal::BrokeUserspace(format!(
                "DONE after {written} of the {} octets offered",
                offer.size
            )));
        }
        self.streams.insert(
            frame.stream,
            StreamState::ReceivedAwaitingFsck { offer, receiving },
        );
        Ok(())
    }

    async fn on_fsck(&mut self, frame: Frame) -> Step {
        let fsck: payload::Fsck = match frame.parse() {
            Ok(fsck) => fsck,
            Err(e) => return self.cpp(frame.stream, e.to_string()).await,
        };
        let Some(StreamState::ReceivedAwaitingFsck {
            offer,
            mut receiving,
        }) = self.streams.remove(&frame.stream)
        else {
            return Err(Fatal::BrokeUserspace(format!(
                "FSCK on stream {} before DONE",
                frame.stream
            )));
        };

        let Some(algorithm) = Algorithm::parse(&fsck.algorithm) else {
            receiving.discard().await;
            return self
                .cpp(
                    frame.stream,
                    format!("digest algorithm {} is not supported", fsck.algorithm),
                )
                .await;
        };
        if fsck.total_size != offer.size {
            receiving.discard().await;
            return Err(Fatal::BrokeUserspace(format!(
                "FSCK claims {} octets but the offer promised {}",
                fsck.total_size, offer.size
            )));
        }

        if !receiving.verify(algorithm, &fsck.digest) {
            receiving.discard().await;
            self.send(Frame::bare(Opcode::Corrupt, frame.stream))
                .await?;
            self.emit(Event::TransferFailed {
                connection: self.id,
                stream: frame.stream.0,
                direction: Direction::Incoming,
                reason: format!("{} digest did not match", algorithm.name()),
            })
            .await;
            return Ok(());
        }

        // RFC R8: the object is committed before CLEAN, so CLEAN never promises a file that
        // failed to land.
        let committed = receiving.commit().await;
        let path = match committed {
            Ok(Received::File(path)) => Some(path),
            Ok(Received::Clipboard(text)) => {
                let clipboard = self.shared.clipboard.clone();
                if let Ok(Err(e)) =
                    tokio::task::spawn_blocking(move || clipboard.write_text(text)).await
                {
                    tracing::warn!(error = %e, "clipboard rejected the received text");
                }
                None
            }
            Err(e) => {
                self.send(Frame::bare(Opcode::Corrupt, frame.stream))
                    .await?;
                self.emit(Event::TransferFailed {
                    connection: self.id,
                    stream: frame.stream.0,
                    direction: Direction::Incoming,
                    reason: e.to_string(),
                })
                .await;
                return Ok(());
            }
        };

        self.send(Frame::bare(Opcode::Clean, frame.stream)).await?;
        self.emit(Event::TransferComplete {
            connection: self.id,
            stream: frame.stream.0,
            direction: Direction::Incoming,
            kind: offer.kind,
            path,
        })
        .await;
        Ok(())
    }

    async fn on_verification(&mut self, frame: Frame) -> Step {
        let Some(StreamState::AwaitingVerification { kind }) = self.streams.remove(&frame.stream)
        else {
            return Err(Fatal::BrokeUserspace(format!(
                "{} on stream {}, which is not awaiting verification",
                frame.opcode, frame.stream
            )));
        };
        let event = if frame.opcode == Opcode::Clean {
            Event::TransferComplete {
                connection: self.id,
                stream: frame.stream.0,
                direction: Direction::Outgoing,
                kind,
                path: None,
            }
        } else {
            Event::TransferFailed {
                connection: self.id,
                stream: frame.stream.0,
                direction: Direction::Outgoing,
                reason: "the receiver reported CORRUPT".into(),
            }
        };
        self.emit(event).await;
        Ok(())
    }

    async fn on_refusal(&mut self, frame: Frame) -> Step {
        let verdict = if frame.opcode == Opcode::BigDiff {
            Verdict::BigDiff {
                max_size: frame.parse_or_default::<payload::BigDiff>().max_size,
            }
        } else {
            Verdict::FuckOff {
                reason: frame.parse_or_default::<payload::FuckOff>().reason,
            }
        };

        match self.streams.remove(&frame.stream) {
            Some(StreamState::OfferedByUs { .. }) => {
                self.emit(Event::OfferResolved {
                    connection: self.id,
                    stream: frame.stream.0,
                    direction: Direction::Incoming,
                    verdict,
                })
                .await;
                Ok(())
            }
            // RFC R6: the offering peer withdraws an object it can no longer deliver.
            Some(StreamState::OfferedToUs { .. }) => {
                self.emit(Event::TransferFailed {
                    connection: self.id,
                    stream: frame.stream.0,
                    direction: Direction::Incoming,
                    reason: "the sender withdrew the offer".into(),
                })
                .await;
                Ok(())
            }
            Some(StreamState::ReceivingFromPeer { receiving, .. })
            | Some(StreamState::ReceivedAwaitingFsck { receiving, .. }) => {
                receiving.discard().await;
                self.emit(Event::TransferFailed {
                    connection: self.id,
                    stream: frame.stream.0,
                    direction: Direction::Incoming,
                    reason: "the sender withdrew the transfer".into(),
                })
                .await;
                Ok(())
            }
            _ => Err(Fatal::BrokeUserspace(format!(
                "{} on stream {}, which carries no offer",
                frame.opcode, frame.stream
            ))),
        }
    }

    async fn on_shut_up(&mut self, frame: Frame) -> Step {
        let notice: payload::ShutUp = match frame.parse() {
            Ok(notice) => notice,
            Err(e) => return self.cpp(frame.stream, e.to_string()).await,
        };
        self.restraint.apply(&notice);
        self.emit(Event::RateLimited {
            connection: self.id,
            direction: Direction::Incoming,
            retry_after_ms: notice.retry_after_ms(),
            scope: notice.scope,
        })
        .await;

        if let Some(StreamState::OfferedByUs { .. }) = self.streams.remove(&frame.stream) {
            self.emit(Event::OfferResolved {
                connection: self.id,
                stream: frame.stream.0,
                direction: Direction::Incoming,
                verdict: Verdict::FuckOff {
                    reason: Some("rate limited".into()),
                },
            })
            .await;
        }
        Ok(())
    }

    async fn on_chat(&mut self, frame: Frame) -> Step {
        let message: Lkml = match frame.parse() {
            Ok(message) => message,
            Err(e) => return self.cpp(frame.stream, e.to_string()).await,
        };
        if let Err(e) = message.validate() {
            return self.cpp(frame.stream, e.to_string()).await;
        }

        match self.peer_chat {
            Some(existing) if existing == frame.stream => {}
            Some(existing) => {
                return Err(Fatal::BrokeUserspace(format!(
                    "stream {} opens a second conversation; {existing} already carries one",
                    frame.stream
                )))
            }
            None => {
                self.peer_streams
                    .accept_new(frame.stream)
                    .map_err(|v| Fatal::BrokeUserspace(v.to_string()))?;
                self.peer_chat = Some(frame.stream);
                self.streams.insert(frame.stream, StreamState::Chat);
                if self.role == Role::Responder {
                    if let Some(ours) = self.chat_send.replace(frame.stream) {
                        self.streams.remove(&ours);
                    }
                }
            }
        }

        if let Err(delay) = self.limiter.chat.take() {
            return self.rate_limit(frame.stream, delay, Opcode::Lkml).await;
        }

        self.emit(Event::Chat {
            connection: self.id,
            stream: frame.stream.0,
            from: self.address.ip(),
            fingerprint: self.fingerprint,
            message_id: sanitize_display(&message.message_id, 128),
            timestamp: message.timestamp,
            text: message.text,
            tags: message.tags,
        })
        .await;
        Ok(())
    }

    async fn on_tick(&mut self) -> Step {
        let timeouts = &self.shared.config.timeouts;
        if self.last_activity.elapsed() > timeouts.idle() {
            return Err(Fatal::Closed("peer went silent".into()));
        }

        // RFC R11: a header that arrives without its payload must not hold the buffer open.
        // Measured from the last complete frame, so a busy connection never trips it.
        if self.framed.read_buffer().is_empty() {
            self.partial_since = None;
        } else {
            let started = *self.partial_since.get_or_insert_with(Instant::now);
            if started.elapsed() > timeouts.frame() {
                return Err(Fatal::Closed("a frame stalled part way through".into()));
            }
        }
        if self.last_activity.elapsed() > timeouts.keepalive() {
            if self.keepalives_outstanding >= 3 {
                return Err(Fatal::Closed("three TUX went unanswered".into()));
            }
            self.keepalives_outstanding += 1;
            self.send(Frame::control(Opcode::Tux)).await?;
        }

        let stale: Vec<StreamId> = self
            .streams
            .iter()
            .filter_map(|(id, state)| match state {
                StreamState::OfferedToUs { since, .. } if since.elapsed() > OFFER_PATIENCE => {
                    Some(*id)
                }
                _ => None,
            })
            .collect();
        for stream in stale {
            self.answer_offer(
                stream,
                Verdict::FuckOff {
                    reason: Some("nobody answered the offer".into()),
                },
            )
            .await?;
        }
        Ok(())
    }

    /// A peer that outruns its budget is told to slow down rather than disconnected (RFC R11).
    async fn throttle(&mut self) -> Step {
        let Err(delay) = self.limiter.frames.take() else {
            return Ok(());
        };
        let cooled = self
            .last_shut_up
            .map(|at| at.elapsed() >= SHUT_UP_COOLDOWN)
            .unwrap_or(true);
        if cooled {
            self.last_shut_up = Some(Instant::now());
            self.note_shut_up()?;
            let notice = shut_up(delay, ShutUpScope::Connection, None);
            self.send(Frame::control_cbor(Opcode::ShutUp, &notice)?)
                .await?;
            self.emit(Event::RateLimited {
                connection: self.id,
                direction: Direction::Outgoing,
                retry_after_ms: notice.retry_after_ms,
                scope: ShutUpScope::Connection,
            })
            .await;
        }
        tokio::time::sleep(delay).await;
        Ok(())
    }

    async fn rate_limit(&mut self, stream: StreamId, delay: Duration, kind: Opcode) -> Step {
        self.note_shut_up()?;
        let notice = shut_up(delay, ShutUpScope::Kind, Some(kind.name()));
        self.send(Frame::cbor(Opcode::ShutUp, stream, &notice)?)
            .await?;
        self.emit(Event::RateLimited {
            connection: self.id,
            direction: Direction::Outgoing,
            retry_after_ms: notice.retry_after_ms,
            scope: ShutUpScope::Kind,
        })
        .await;
        Ok(())
    }

    /// `NICE-1.md` §14: a peer that keeps ignoring `SHUT_UP` eventually gets `NVIDIA`.
    fn note_shut_up(&mut self) -> Step {
        self.shut_ups_sent += 1;
        if self.shut_ups_sent > MAX_SHUT_UPS {
            return Err(Fatal::Nvidia(format!("ignored {MAX_SHUT_UPS} rate limits")));
        }
        Ok(())
    }

    async fn held_back(&self, kind: Opcode) -> bool {
        let Some(delay) = self.restraint.blocked(kind.name()) else {
            return false;
        };
        self.emit(Event::RateLimited {
            connection: self.id,
            direction: Direction::Incoming,
            retry_after_ms: delay.as_millis() as u64,
            scope: ShutUpScope::Kind,
        })
        .await;
        true
    }

    async fn withdraw(&mut self, stream: StreamId, reason: impl Into<String>) -> Step {
        let reason = reason.into();
        self.streams.remove(&stream);
        self.sends.remove(&stream);
        self.send_queue.retain(|queued| *queued != stream);
        let refusal = payload::FuckOff {
            reason: Some(reason.clone()),
        };
        self.send(Frame::cbor(Opcode::FuckOff, stream, &refusal)?)
            .await?;
        self.emit(Event::TransferFailed {
            connection: self.id,
            stream: stream.0,
            direction: Direction::Outgoing,
            reason,
        })
        .await;
        Ok(())
    }

    /// RFC R9: recovery abandons every stream and resets both allocators.
    async fn reset(&mut self) {
        self.abandon_streams().await;
        self.allocator.reset();
        self.peer_streams.reset();
        self.chat_send = None;
        self.peer_chat = None;
    }

    async fn abandon_streams(&mut self) {
        for state in std::mem::take(&mut self.streams).into_values() {
            match state {
                StreamState::ReceivingFromPeer { receiving, .. }
                | StreamState::ReceivedAwaitingFsck { receiving, .. } => {
                    receiving.discard().await;
                }
                _ => {}
            }
        }
        self.sends.clear();
        self.send_queue.clear();
    }

    fn pending_offers(&self) -> usize {
        self.streams
            .values()
            .filter(|state| matches!(state, StreamState::OfferedToUs { .. }))
            .count()
    }

    /// The store is consulted rather than the flag captured at handshake time, because a
    /// peer can be paired while this connection is already open.
    fn auto_accepts(&self, kind: OfferKind) -> bool {
        let Some(fingerprint) = self.fingerprint else {
            return false;
        };
        self.shared
            .pairing
            .lock()
            .expect("pairing store")
            .get(&fingerprint)
            .map(|peer| peer.auto_accepts(kind))
            .unwrap_or(false)
    }

    async fn cpp(&mut self, stream: StreamId, reason: impl Into<String>) -> Step {
        let reason = reason.into();
        self.send(Frame::cbor(Opcode::Cpp, stream, &Reason::new(&reason))?)
            .await?;
        self.emit(Event::ProtocolError {
            connection: self.id,
            opcode: Opcode::Cpp.name().into(),
            reason,
        })
        .await;
        Ok(())
    }

    async fn report(&mut self, opcode: Opcode, reason: &str) {
        let frame = Frame::control_cbor(opcode, &Reason::new(reason))
            .unwrap_or_else(|_| Frame::control(opcode));
        let _ = self.framed.send(frame).await;
        self.emit(Event::ProtocolError {
            connection: self.id,
            opcode: opcode.name().to_string(),
            reason: reason.to_string(),
        })
        .await;
    }

    async fn warn(&self, message: impl Into<String>) -> Step {
        self.emit(Event::Warning {
            message: message.into(),
        })
        .await;
        Ok(())
    }

    async fn send(&mut self, frame: Frame) -> Step {
        self.framed.send(frame).await.map_err(Fatal::from)
    }

    async fn emit(&self, event: Event) {
        let _ = self.events.send(event).await;
    }
}

enum Wake {
    Command(Option<SessionCommand>),
    Frame(Option<std::result::Result<Incoming, ProtoError>>),
    Tick,
    Send,
}

fn capabilities() -> Vec<String> {
    vec![
        "file".into(),
        "clipboard".into(),
        "chat".into(),
        "blake3".into(),
    ]
}

fn preview_of(text: &str) -> String {
    let mut preview = String::new();
    for c in text.chars().filter(|c| !c.is_control() || *c == '\n') {
        if preview.len() + c.len_utf8() > payload::MAX_PREVIEW {
            break;
        }
        preview.push(c);
    }
    preview
}

async fn expect(framed: &mut Framed<Stream, NiceCodec>, want: Opcode) -> Result<Frame> {
    match framed.next().await {
        Some(Ok(Incoming::Frame(frame))) if frame.opcode == want => {
            if !frame.stream.is_connection() {
                return Err(Error::Rejected(format!(
                    "{want} arrived on stream {}, but the handshake is connection-level",
                    frame.stream
                )));
            }
            Ok(frame)
        }
        Some(Ok(Incoming::Frame(frame))) => Err(Error::Rejected(format!(
            "expected {want}, got {}",
            frame.opcode
        ))),
        Some(Ok(Incoming::Unknown { opcode, .. })) => Err(Error::Rejected(format!(
            "expected {want}, got undefined opcode 0x{opcode:02x}"
        ))),
        Some(Err(e)) => Err(e.into()),
        None => Err(Error::Rejected(format!(
            "peer closed the connection before {want}"
        ))),
    }
}
