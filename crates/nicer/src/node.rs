use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use nicer_proto::{Role, StreamId};
use serde::{Deserialize, Serialize};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{broadcast, mpsc, oneshot};

use crate::clipboard::Clipboard;
use crate::config::{Config, Mode, PLAINTEXT_WARNING};
use crate::discovery::Discovery;
use crate::error::Result;
use crate::event::{Command, ConnectionId, ConnectionSummary, Event, Status};
use crate::identity::{Fingerprint, Identity};
use crate::pairing::{PairedPeer, PairingStore};
use crate::paths::Paths;
use crate::session::{Session, SessionCommand, SessionInfo, Shared};
use crate::transport::Transport;

const EVENT_BUFFER: usize = 256;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "reply", rename_all = "snake_case")]
pub enum Reply {
    Ok,
    Status(Status),
    Peers { peers: Vec<PairedPeer> },
    Connections { connections: Vec<ConnectionSummary> },
    Connected { connection: ConnectionId },
    Error { message: String },
}

impl Reply {
    fn error(message: impl std::fmt::Display) -> Self {
        Reply::Error {
            message: message.to_string(),
        }
    }
}

pub struct Request {
    pub command: Command,
    pub reply: oneshot::Sender<Reply>,
}

/// The handle a control interface holds: commands in, events out.
#[derive(Clone)]
pub struct NodeHandle {
    requests: mpsc::Sender<Request>,
    events: broadcast::Sender<Event>,
}

impl NodeHandle {
    pub async fn call(&self, command: Command) -> Reply {
        let (reply, answer) = oneshot::channel();
        if self
            .requests
            .send(Request { command, reply })
            .await
            .is_err()
        {
            return Reply::error("the node has stopped");
        }
        answer
            .await
            .unwrap_or_else(|_| Reply::error("the node dropped the request"))
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.events.subscribe()
    }
}

enum Target {
    Accepted(TcpStream, SocketAddr),
    Dial(SocketAddr),
}

enum Internal {
    Inbound(TcpStream, SocketAddr),
    Established {
        session: Box<Session>,
        commands: mpsc::Sender<SessionCommand>,
        reply: Option<oneshot::Sender<Reply>>,
    },
    Failed {
        address: SocketAddr,
        expected: Option<Fingerprint>,
        reason: String,
        reply: Option<oneshot::Sender<Reply>>,
    },
    Closed(ConnectionId, String),
}

struct Connection {
    info: SessionInfo,
    commands: mpsc::Sender<SessionCommand>,
}

pub struct Node {
    shared: Arc<Shared>,
    transport: Transport,
    listen: SocketAddr,
    connections: HashMap<ConnectionId, Connection>,
    next_id: u64,
    events_in: mpsc::Sender<Event>,
    events_out: broadcast::Sender<Event>,
    internal: mpsc::Sender<Internal>,
    discovery: Option<Discovery>,
}

impl Node {
    pub fn build(
        config: Config,
        paths: &Paths,
        clipboard: Box<dyn Clipboard>,
    ) -> Result<(Self, NodeHandle, NodeInbox)> {
        paths.create_all()?;
        let identity = Arc::new(Identity::load_or_generate(
            &paths.identity_file(),
            &config.device_name,
        )?);
        let pairing = Arc::new(Mutex::new(PairingStore::load(&paths.peers_file())?));

        let transport = match config.mode() {
            Mode::Secure => Transport::secure(identity.clone(), config.keylog.clone())?,
            Mode::Plaintext => Transport::Plaintext,
        };

        let listen = config.listen;
        let shared = Arc::new(Shared {
            config,
            identity,
            pairing,
            clipboard: Arc::from(clipboard),
        });

        let (requests_tx, requests_rx) = mpsc::channel(64);
        let (events_in, events_rx) = mpsc::channel(EVENT_BUFFER);
        let (events_out, _) = broadcast::channel(EVENT_BUFFER);
        let (internal_tx, internal_rx) = mpsc::channel(64);

        let node = Node {
            shared,
            transport,
            listen,
            connections: HashMap::new(),
            next_id: 1,
            events_in,
            events_out: events_out.clone(),
            internal: internal_tx,
            discovery: None,
        };
        let handle = NodeHandle {
            requests: requests_tx,
            events: events_out,
        };
        let inbox = NodeInbox {
            requests: requests_rx,
            events: events_rx,
            internal: internal_rx,
        };
        Ok((node, handle, inbox))
    }

    pub fn fingerprint(&self) -> Fingerprint {
        self.shared.identity.fingerprint()
    }

    pub async fn run(mut self, mut inbox: NodeInbox) -> Result<()> {
        let listener = TcpListener::bind(self.listen).await?;
        let bound = listener.local_addr()?;
        let accepts = self.internal.clone();
        tokio::spawn(async move {
            loop {
                match listener.accept().await {
                    Ok((stream, address)) => {
                        if accepts
                            .send(Internal::Inbound(stream, address))
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "accept failed");
                    }
                }
            }
        });

        if !self.shared.config.is_secure() {
            self.publish(Event::Warning {
                message: PLAINTEXT_WARNING.into(),
            });
        }

        if self.shared.config.discovery {
            match Discovery::start(
                &self.shared.config.device_name,
                bound.port(),
                self.shared.config.is_secure(),
                self.fingerprint(),
                self.events_in.clone(),
            ) {
                Ok(discovery) => self.discovery = Some(discovery),
                Err(e) => self.publish(Event::Warning {
                    message: format!("discovery is unavailable: {e}"),
                }),
            }
        }

        self.publish(Event::Listening {
            address: bound,
            transport: self.transport.mode(),
            device: self.shared.config.device_name.clone(),
            fingerprint: self.fingerprint(),
        });

        loop {
            tokio::select! {
                request = inbox.requests.recv() => match request {
                    Some(request) => self.on_request(request).await,
                    None => break,
                },
                event = inbox.events.recv() => match event {
                    Some(event) => self.publish(event),
                    None => break,
                },
                internal = inbox.internal.recv() => match internal {
                    Some(message) => self.on_internal(message).await,
                    None => break,
                },
            }
        }

        if let Some(discovery) = &self.discovery {
            discovery.shutdown();
        }
        Ok(())
    }

    fn publish(&self, event: Event) {
        let _ = self.events_out.send(event);
    }

    async fn on_internal(&mut self, message: Internal) {
        match message {
            Internal::Inbound(stream, address) => self.accept(stream, address),
            Internal::Established {
                session,
                commands,
                reply,
            } => {
                let id = self.adopt(*session, commands);
                if let Some(reply) = reply {
                    let _ = reply.send(Reply::Connected { connection: id });
                }
            }
            Internal::Failed {
                address,
                expected,
                reason,
                reply,
            } => {
                // RFC R5: a stored identity that no longer matches is reported, never
                // re-prompted.
                if let Some(expected) = expected {
                    self.publish(Event::IdentityChanged {
                        address,
                        expected,
                        reason: reason.clone(),
                    });
                }
                if let Some(reply) = reply {
                    let _ = reply.send(Reply::error(&reason));
                } else {
                    self.publish(Event::Warning {
                        message: format!("{address}: {reason}"),
                    });
                }
            }
            Internal::Closed(id, reason) => {
                self.connections.remove(&id);
                self.publish(Event::Disconnected {
                    connection: id,
                    reason,
                });
            }
        }
    }

    fn accept(&mut self, stream: TcpStream, address: SocketAddr) {
        let already = self
            .connections
            .values()
            .filter(|connection| connection.info.address.ip() == address.ip())
            .count();
        if already >= self.shared.config.limits.max_connections_per_address {
            self.publish(Event::Warning {
                message: format!("{address} has too many connections open"),
            });
            return;
        }
        self.spawn_handshake(Target::Accepted(stream, address), None, None);
    }

    fn spawn_handshake(
        &mut self,
        target: Target,
        expected: Option<Fingerprint>,
        reply: Option<oneshot::Sender<Reply>>,
    ) {
        let id = ConnectionId(self.next_id);
        self.next_id += 1;

        let (commands_tx, commands_rx) = mpsc::channel(32);
        let shared = self.shared.clone();
        let transport = self.transport.clone();
        let events = self.events_in.clone();
        let internal = self.internal.clone();
        let patience = shared.config.timeouts.handshake();

        let address = match &target {
            Target::Accepted(_, address) | Target::Dial(address) => *address,
        };

        tokio::spawn(async move {
            let attempt = tokio::time::timeout(patience, async {
                let (stream, role) = match target {
                    Target::Accepted(stream, _) => {
                        (transport.accept(stream).await?, Role::Responder)
                    }
                    Target::Dial(address) => {
                        let stream = TcpStream::connect(address).await?;
                        (transport.connect(stream, expected).await?, Role::Initiator)
                    }
                };
                Session::establish(id, stream, address, role, shared, events, commands_rx).await
            })
            .await;

            let message = match attempt {
                Ok(Ok(session)) => Internal::Established {
                    session: Box::new(session),
                    commands: commands_tx,
                    reply,
                },
                Ok(Err(e)) => Internal::Failed {
                    address,
                    expected,
                    reason: e.to_string(),
                    reply,
                },
                Err(_) => Internal::Failed {
                    address,
                    expected,
                    reason: "the handshake did not finish in time".into(),
                    reply,
                },
            };
            let _ = internal.send(message).await;
        });
    }

    fn adopt(&mut self, session: Session, commands: mpsc::Sender<SessionCommand>) -> ConnectionId {
        let info = session.info();
        let id = session.id();

        self.publish(Event::Connected {
            connection: id,
            address: info.address,
            device: info.device.clone(),
            transport: info.transport,
            direction: info.direction,
            fingerprint: info.fingerprint,
            paired: info.paired,
        });

        if let (Some(fingerprint), false) = (info.fingerprint, info.paired) {
            self.publish(Event::PairingRequired {
                connection: id,
                address: info.address,
                device: info.device.clone(),
                fingerprint,
                short: fingerprint.short(),
            });
        }

        self.connections.insert(id, Connection { info, commands });

        let internal = self.internal.clone();
        tokio::spawn(async move {
            let reason = session.run().await;
            let _ = internal.send(Internal::Closed(id, reason)).await;
        });

        id
    }

    /// A command whose answer depends on a handshake hands its reply channel to the task
    /// that finishes it; answering from this loop would deadlock it.
    async fn on_request(&mut self, request: Request) {
        let Request { command, reply } = request;
        if let Command::Connect { address } = command {
            self.connect(address, reply);
            return;
        }
        let answer = self.dispatch(command).await;
        let _ = reply.send(answer);
    }

    async fn dispatch(&mut self, command: Command) -> Reply {
        match command {
            Command::Status => Reply::Status(self.status()),
            Command::ListPeers => Reply::Peers {
                peers: self
                    .shared
                    .pairing
                    .lock()
                    .expect("pairing store")
                    .list()
                    .cloned()
                    .collect(),
            },
            Command::ListConnections => Reply::Connections {
                connections: self
                    .connections
                    .iter()
                    .map(|(id, connection)| summary(*id, &connection.info))
                    .collect(),
            },
            Command::Connect { .. } => unreachable!("handled by on_request"),
            Command::Disconnect { connection } => {
                self.forward(
                    connection,
                    SessionCommand::Close("disconnected locally".into()),
                )
                .await
            }
            Command::Pair {
                fingerprint,
                device,
            } => {
                let device = device.unwrap_or_else(|| self.device_for(&fingerprint));
                let paired = self
                    .shared
                    .pairing
                    .lock()
                    .expect("pairing store")
                    .pair(fingerprint, &device)
                    .map(|_| ());
                match paired {
                    Ok(()) => {
                        self.mark_paired(&fingerprint, true);
                        Reply::Ok
                    }
                    Err(e) => Reply::error(e),
                }
            }
            Command::Unpair { fingerprint } => {
                let unpaired = self
                    .shared
                    .pairing
                    .lock()
                    .expect("pairing store")
                    .unpair(&fingerprint);
                match unpaired {
                    Ok(true) => {
                        self.mark_paired(&fingerprint, false);
                        Reply::Ok
                    }
                    Ok(false) => Reply::error(format!("{} was not paired", fingerprint.short())),
                    Err(e) => Reply::error(e),
                }
            }
            Command::AutoAccept {
                fingerprint,
                kind,
                enabled,
            } => match self
                .shared
                .pairing
                .lock()
                .expect("pairing store")
                .set_auto_accept(&fingerprint, kind, enabled)
            {
                Ok(true) => Reply::Ok,
                Ok(false) => Reply::error(format!(
                    "{} must be paired before it can auto-accept",
                    fingerprint.short()
                )),
                Err(e) => Reply::error(e),
            },
            Command::SendFile { connection, path } => {
                self.forward(connection, SessionCommand::OfferFile(path))
                    .await
            }
            Command::SendClipboard { connection, text } => {
                self.forward(connection, SessionCommand::OfferClipboard(text))
                    .await
            }
            Command::Respond {
                connection,
                stream,
                verdict,
            } => {
                self.forward(
                    connection,
                    SessionCommand::Respond {
                        stream: StreamId(stream),
                        verdict,
                    },
                )
                .await
            }
            Command::Chat { connection, text } => {
                self.forward(connection, SessionCommand::Chat(text)).await
            }
            Command::Resync { connection } => {
                self.forward(connection, SessionCommand::Resync).await
            }
        }
    }

    fn connect(&mut self, address: SocketAddr, reply: oneshot::Sender<Reply>) {
        let expected = self.expected_identity(address);
        self.spawn_handshake(Target::Dial(address), expected, Some(reply));
    }

    /// A peer already paired at this address is pinned, so an identity change fails the
    /// handshake rather than reaching the user as a fresh pairing prompt (RFC R5).
    fn expected_identity(&self, address: SocketAddr) -> Option<Fingerprint> {
        self.shared
            .pairing
            .lock()
            .expect("pairing store")
            .list()
            .find(|peer| peer.last_address == Some(address.ip()))
            .map(|peer| peer.fingerprint)
    }

    fn device_for(&self, fingerprint: &Fingerprint) -> String {
        self.connections
            .values()
            .find(|connection| connection.info.fingerprint.as_ref() == Some(fingerprint))
            .map(|connection| connection.info.device.clone())
            .unwrap_or_else(|| fingerprint.short())
    }

    fn mark_paired(&mut self, fingerprint: &Fingerprint, paired: bool) {
        for connection in self.connections.values_mut() {
            if connection.info.fingerprint.as_ref() == Some(fingerprint) {
                connection.info.paired = paired;
            }
        }
    }

    async fn forward(&mut self, id: ConnectionId, command: SessionCommand) -> Reply {
        let Some(connection) = self.connections.get(&id) else {
            return Reply::error(format!("connection {id} is not open"));
        };
        match connection.commands.send(command).await {
            Ok(()) => Reply::Ok,
            Err(_) => Reply::error(format!("connection {id} has closed")),
        }
    }

    fn status(&self) -> Status {
        Status {
            device: self.shared.config.device_name.clone(),
            fingerprint: self.fingerprint(),
            short: self.fingerprint().short(),
            listen: self.listen,
            transport: self.transport.mode(),
            discovery: self.discovery.is_some(),
            download_dir: self.shared.config.download_dir.clone(),
            connections: self.connections.len(),
            paired_peers: self
                .shared
                .pairing
                .lock()
                .expect("pairing store")
                .list()
                .count(),
        }
    }
}

pub struct NodeInbox {
    requests: mpsc::Receiver<Request>,
    events: mpsc::Receiver<Event>,
    internal: mpsc::Receiver<Internal>,
}

fn summary(id: ConnectionId, info: &SessionInfo) -> ConnectionSummary {
    ConnectionSummary {
        connection: id,
        address: info.address,
        device: info.device.clone(),
        transport: info.transport,
        direction: info.direction,
        fingerprint: info.fingerprint,
        paired: info.paired,
    }
}
