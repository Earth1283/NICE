use bytes::{Buf, Bytes, BytesMut};
use tokio_util::codec::{Decoder, Encoder};

use crate::error::ProtoError;
use crate::frame::{Frame, HEADER_LEN, MAGIC, VERSION};
use crate::opcode::Opcode;
use crate::stream::StreamId;

/// A decoded frame. An opcode this revision does not define is preserved rather than
/// rejected, so the session can answer `CPP` and keep the connection (RFC R1).
#[derive(Clone, Debug)]
pub enum Incoming {
    Frame(Frame),
    Unknown {
        opcode: u8,
        stream: StreamId,
        payload: Bytes,
    },
}

impl Incoming {
    pub fn stream(&self) -> StreamId {
        match self {
            Incoming::Frame(frame) => frame.stream,
            Incoming::Unknown { stream, .. } => *stream,
        }
    }
}

#[derive(Debug)]
pub struct NiceCodec {
    max_incoming: u32,
    max_outgoing: u32,
}

impl NiceCodec {
    pub fn new(max_incoming: u32) -> Self {
        Self {
            max_incoming,
            max_outgoing: u32::MAX,
        }
    }

    /// Applied once the peer's `HELLO` or `MERGED` states what it will accept (RFC R2).
    pub fn set_peer_limit(&mut self, max_outgoing: u32) {
        self.max_outgoing = max_outgoing;
    }

    pub fn peer_limit(&self) -> u32 {
        self.max_outgoing
    }
}

impl Decoder for NiceCodec {
    type Item = Incoming;
    type Error = ProtoError;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        if src.len() < HEADER_LEN {
            src.reserve(HEADER_LEN - src.len());
            return Ok(None);
        }

        let header = &src[..HEADER_LEN];
        if header[0] != MAGIC {
            return Err(ProtoError::BadMagic(header[0]));
        }
        if header[1] != VERSION {
            return Err(ProtoError::BadVersion(header[1]));
        }
        if header[3] != 0 {
            return Err(ProtoError::ReservedNotZero(header[3]));
        }

        let opcode = header[2];
        let stream = StreamId(u32::from_be_bytes(header[4..8].try_into().unwrap()));
        let len = u32::from_be_bytes(header[8..12].try_into().unwrap());

        if len > self.max_incoming {
            return Err(ProtoError::FrameTooLarge {
                len,
                limit: self.max_incoming,
            });
        }

        let total = HEADER_LEN + len as usize;
        if src.len() < total {
            src.reserve(total - src.len());
            return Ok(None);
        }

        let mut frame = src.split_to(total);
        frame.advance(HEADER_LEN);
        let payload = frame.freeze();

        Ok(Some(match Opcode::from_u8(opcode) {
            Some(opcode) => Incoming::Frame(Frame::new(opcode, stream, payload)),
            None => Incoming::Unknown {
                opcode,
                stream,
                payload,
            },
        }))
    }
}

impl Encoder<Frame> for NiceCodec {
    type Error = ProtoError;

    fn encode(&mut self, frame: Frame, dst: &mut BytesMut) -> Result<(), Self::Error> {
        let len = frame.payload.len();
        if len > self.max_outgoing as usize {
            return Err(ProtoError::FrameTooLarge {
                len: len as u32,
                limit: self.max_outgoing,
            });
        }
        frame.encode_into(dst);
        Ok(())
    }
}
