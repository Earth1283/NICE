use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::mpsc;

use crate::error::Result;
use crate::event::{Command, Event};
use crate::node::{NodeHandle, Reply};

const OUTBOX: usize = 512;

#[derive(Deserialize)]
struct Envelope {
    #[serde(default)]
    id: Option<serde_json::Value>,
    #[serde(flatten)]
    command: Command,
}

#[derive(Serialize)]
struct Answer {
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<serde_json::Value>,
    #[serde(flatten)]
    reply: Reply,
}

/// Newline-delimited JSON: one command object per line in, one reply or event object per
/// line out. Events arrive unsolicited and carry no `id`.
pub async fn serve(endpoint: &Path, handle: NodeHandle) -> Result<()> {
    let mut listener = platform::bind(endpoint)?;
    tracing::info!(endpoint = %endpoint.display(), "control interface ready");
    loop {
        let connection = platform::accept(&mut listener).await?;
        let handle = handle.clone();
        tokio::spawn(async move {
            if let Err(e) = client(connection, handle).await {
                tracing::debug!(error = %e, "control client ended");
            }
        });
    }
}

async fn client(connection: platform::Connection, handle: NodeHandle) -> Result<()> {
    let (reader, mut writer) = tokio::io::split(connection);
    let (outbox, mut outgoing) = mpsc::channel::<String>(OUTBOX);

    let pump = tokio::spawn(async move {
        while let Some(line) = outgoing.recv().await {
            if writer.write_all(line.as_bytes()).await.is_err()
                || writer.write_all(b"\n").await.is_err()
            {
                break;
            }
        }
    });

    let mut events = handle.subscribe();
    let event_outbox = outbox.clone();
    let broadcaster = tokio::spawn(async move {
        loop {
            match events.recv().await {
                Ok(event) => match serde_json::to_string(&event) {
                    Ok(line) => {
                        if event_outbox.send(line).await.is_err() {
                            break;
                        }
                    }
                    Err(e) => tracing::error!(error = %e, "an event could not be serialised"),
                },
                Err(tokio::sync::broadcast::error::RecvError::Lagged(missed)) => {
                    let notice = Event::Warning {
                        message: format!("{missed} events were dropped; this client is behind"),
                    };
                    if let Ok(line) = serde_json::to_string(&notice) {
                        if event_outbox.send(line).await.is_err() {
                            break;
                        }
                    }
                }
                Err(_) => break,
            }
        }
    });

    let mut lines = BufReader::new(reader).lines();
    while let Some(line) = lines.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }
        let answer = match serde_json::from_str::<Envelope>(&line) {
            Ok(envelope) => Answer {
                id: envelope.id,
                reply: handle.call(envelope.command).await,
            },
            Err(e) => Answer {
                id: None,
                reply: Reply::Error {
                    message: e.to_string(),
                },
            },
        };
        // A reply that cannot be serialised must still reach the client, or the caller
        // waits forever for an answer that was silently dropped.
        let line = serde_json::to_string(&answer).unwrap_or_else(|e| {
            tracing::error!(error = %e, "a reply could not be serialised");
            format!(r#"{{"reply":"error","message":"reply could not be serialised: {e}"}}"#)
        });
        if outbox.send(line).await.is_err() {
            break;
        }
    }

    drop(outbox);
    broadcaster.abort();
    let _ = pump.await;
    Ok(())
}

#[cfg(unix)]
mod platform {
    use super::*;

    pub type Listener = tokio::net::UnixListener;
    pub type Connection = tokio::net::UnixStream;

    pub fn bind(path: &Path) -> Result<Listener> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        // A socket left behind by a crashed daemon would otherwise make bind fail forever.
        match std::os::unix::net::UnixStream::connect(path) {
            Ok(_) => {
                return Err(crate::error::Error::Config(format!(
                    "{} is already served by a running daemon",
                    path.display()
                )))
            }
            Err(_) => {
                let _ = std::fs::remove_file(path);
            }
        }
        let listener = Listener::bind(path)?;
        std::fs::set_permissions(path, permissions())?;
        Ok(listener)
    }

    fn permissions() -> std::fs::Permissions {
        use std::os::unix::fs::PermissionsExt;
        std::fs::Permissions::from_mode(0o600)
    }

    pub async fn accept(listener: &mut Listener) -> Result<Connection> {
        let (connection, _) = listener.accept().await?;
        Ok(connection)
    }
}

#[cfg(windows)]
mod platform {
    use super::*;
    use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};

    pub type Connection = NamedPipeServer;

    pub struct Listener {
        name: String,
        pending: NamedPipeServer,
    }

    pub fn bind(path: &Path) -> Result<Listener> {
        let name = path.to_string_lossy().into_owned();
        let pending = ServerOptions::new()
            .first_pipe_instance(true)
            .create(&name)?;
        Ok(Listener { name, pending })
    }

    pub async fn accept(listener: &mut Listener) -> Result<Connection> {
        listener.pending.connect().await?;
        let next = ServerOptions::new().create(&listener.name)?;
        Ok(std::mem::replace(&mut listener.pending, next))
    }
}

pub fn default_endpoint(runtime_dir: &Path) -> PathBuf {
    if cfg!(windows) {
        PathBuf::from(r"\\.\pipe\nicer")
    } else {
        runtime_dir.join("nicer.sock")
    }
}
