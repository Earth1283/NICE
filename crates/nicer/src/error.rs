use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Proto(#[from] nicer_proto::ProtoError),

    #[error("configuration: {0}")]
    Config(String),

    #[error("identity: {0}")]
    Identity(String),

    #[error("certificate: {0}")]
    Certificate(String),

    #[error("tls: {0}")]
    Tls(String),

    #[error("clipboard: {0}")]
    Clipboard(String),

    #[error("discovery: {0}")]
    Discovery(String),

    #[error("transfer: {0}")]
    Transfer(String),

    #[error("peer {0} is not connected")]
    NotConnected(String),

    #[error("no such peer: {0}")]
    UnknownPeer(String),

    #[error("no such stream: {0}")]
    UnknownStream(u32),

    #[error("{0}")]
    Rejected(String),
}

impl From<rustls::Error> for Error {
    fn from(e: rustls::Error) -> Self {
        Error::Tls(e.to_string())
    }
}

impl From<rcgen::Error> for Error {
    fn from(e: rcgen::Error) -> Self {
        Error::Certificate(e.to_string())
    }
}
