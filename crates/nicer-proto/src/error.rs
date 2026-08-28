use thiserror::Error;

/// Which NICE/1 error opcode answers a condition, per RFC R12.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Fault {
    Cpp,
    BrokeUserspace,
}

#[derive(Debug, Error)]
pub enum ProtoError {
    #[error("magic 0x{0:02x}, expected 0x69")]
    BadMagic(u8),

    #[error("version 0x{0:02x} is not NICE/1")]
    BadVersion(u8),

    #[error("reserved octet 0x{0:02x} is nonzero")]
    ReservedNotZero(u8),

    #[error("payload of {len} octets exceeds the negotiated limit of {limit}")]
    FrameTooLarge { len: u32, limit: u32 },

    #[error("{opcode} payload is not valid CBOR: {source}")]
    Cbor {
        opcode: &'static str,
        #[source]
        source: ciborium::de::Error<std::io::Error>,
    },

    #[error("DIFF payload of {0} octets is shorter than its 8 octet offset")]
    ShortDiff(usize),

    #[error("{field} is {len} octets, over the limit of {limit}")]
    FieldTooLong {
        field: &'static str,
        len: usize,
        limit: usize,
    },

    #[error("{0} carries no payload but one was expected")]
    MissingPayload(&'static str),

    #[error(transparent)]
    Io(#[from] std::io::Error),
}

impl ProtoError {
    /// Whether the octet stream is still aligned after this error.
    pub fn is_framing_loss(&self) -> bool {
        matches!(
            self,
            ProtoError::BadMagic(_) | ProtoError::FrameTooLarge { .. } | ProtoError::Io(_)
        )
    }

    pub fn fault(&self) -> Fault {
        Fault::Cpp
    }
}
