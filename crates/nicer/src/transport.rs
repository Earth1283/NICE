use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use nicer_proto::payload::TransportMode;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;
use tokio_rustls::{TlsAcceptor, TlsConnector};

use crate::error::{Error, Result};
use crate::identity::{Fingerprint, Identity};
use crate::tls;

pub enum Stream {
    Plain(TcpStream),
    Tls(Box<tokio_rustls::TlsStream<TcpStream>>),
}

impl Stream {
    pub fn mode(&self) -> TransportMode {
        match self {
            Stream::Plain(_) => TransportMode::Plaintext,
            Stream::Tls(_) => TransportMode::Secure,
        }
    }

    /// The authenticated identity, which exists only under SECURE (RFC R4).
    pub fn peer_fingerprint(&self) -> Result<Option<Fingerprint>> {
        let Stream::Tls(tls) = self else {
            return Ok(None);
        };
        let certificate = tls
            .get_ref()
            .1
            .peer_certificates()
            .and_then(<[_]>::first)
            .ok_or_else(|| Error::Tls("peer completed TLS without a certificate".into()))?;
        Ok(Some(tls::fingerprint_of(certificate)?))
    }
}

macro_rules! project {
    ($self:expr) => {
        match $self.get_mut() {
            Stream::Plain(inner) => Pin::new(inner) as Pin<&mut (dyn AsyncWriteRead + Send)>,
            Stream::Tls(inner) => Pin::new(inner.as_mut()) as Pin<&mut (dyn AsyncWriteRead + Send)>,
        }
    };
}

trait AsyncWriteRead: AsyncRead + AsyncWrite {}
impl<T: AsyncRead + AsyncWrite> AsyncWriteRead for T {}

impl AsyncRead for Stream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        project!(self).poll_read(cx, buf)
    }
}

impl AsyncWrite for Stream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        project!(self).poll_write(cx, buf)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        project!(self).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        project!(self).poll_shutdown(cx)
    }
}

/// Builds the transport for both directions. A SECURE listener never falls back to
/// PLAINTEXT, and a PLAINTEXT one never upgrades (RFC R14).
#[derive(Clone)]
pub enum Transport {
    Secure {
        acceptor: TlsAcceptor,
        identity: Arc<Identity>,
        keylog: Option<std::path::PathBuf>,
    },
    Plaintext,
}

impl Transport {
    pub fn secure(identity: Arc<Identity>, keylog: Option<std::path::PathBuf>) -> Result<Self> {
        let acceptor =
            TlsAcceptor::from(Arc::new(tls::server_config(&identity, keylog.as_deref())?));
        Ok(Transport::Secure {
            acceptor,
            identity,
            keylog,
        })
    }

    pub fn mode(&self) -> TransportMode {
        match self {
            Transport::Secure { .. } => TransportMode::Secure,
            Transport::Plaintext => TransportMode::Plaintext,
        }
    }

    pub async fn accept(&self, stream: TcpStream) -> Result<Stream> {
        stream.set_nodelay(true)?;
        match self {
            Transport::Plaintext => Ok(Stream::Plain(stream)),
            Transport::Secure { acceptor, .. } => {
                let tls = acceptor.accept(stream).await?;
                Ok(Stream::Tls(Box::new(tls.into())))
            }
        }
    }

    pub async fn connect(
        &self,
        stream: TcpStream,
        expected: Option<Fingerprint>,
    ) -> Result<Stream> {
        stream.set_nodelay(true)?;
        match self {
            Transport::Plaintext => Ok(Stream::Plain(stream)),
            Transport::Secure {
                identity, keylog, ..
            } => {
                let config = tls::client_config(identity, expected, keylog.as_deref())?;
                let connector = TlsConnector::from(Arc::new(config));
                let name = rustls_pki_types::ServerName::try_from(tls::PLACEHOLDER_SERVER_NAME)
                    .map_err(|e| Error::Tls(e.to_string()))?;
                let tls = connector.connect(name, stream).await?;
                Ok(Stream::Tls(Box::new(tls.into())))
            }
        }
    }
}
