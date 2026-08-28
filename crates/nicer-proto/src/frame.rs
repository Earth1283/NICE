use bytes::{Buf, BufMut, Bytes, BytesMut};
use serde::{de::DeserializeOwned, Serialize};

use crate::error::ProtoError;
use crate::opcode::Opcode;
use crate::stream::{StreamId, CONNECTION};

pub const MAGIC: u8 = 0x69;
pub const VERSION: u8 = 0x01;
pub const HEADER_LEN: usize = 12;

/// RFC R2: the payload size every implementation must accept.
pub const MIN_MAX_FRAME_SIZE: u32 = 64 * 1024;
pub const DEFAULT_MAX_FRAME_SIZE: u32 = 1024 * 1024;
pub const HARD_MAX_FRAME_SIZE: u32 = 16 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct Frame {
    pub opcode: Opcode,
    pub stream: StreamId,
    pub payload: Bytes,
}

impl Frame {
    pub fn new(opcode: Opcode, stream: StreamId, payload: Bytes) -> Self {
        Self {
            opcode,
            stream,
            payload,
        }
    }

    pub fn bare(opcode: Opcode, stream: StreamId) -> Self {
        Self::new(opcode, stream, Bytes::new())
    }

    pub fn control(opcode: Opcode) -> Self {
        Self::bare(opcode, CONNECTION)
    }

    pub fn cbor<T: Serialize>(
        opcode: Opcode,
        stream: StreamId,
        value: &T,
    ) -> Result<Self, ProtoError> {
        let mut buf = Vec::new();
        ciborium::into_writer(value, &mut buf)
            .map_err(|e| ProtoError::Io(std::io::Error::other(e)))?;
        Ok(Self::new(opcode, stream, Bytes::from(buf)))
    }

    pub fn control_cbor<T: Serialize>(opcode: Opcode, value: &T) -> Result<Self, ProtoError> {
        Self::cbor(opcode, CONNECTION, value)
    }

    pub fn parse<T: DeserializeOwned>(&self) -> Result<T, ProtoError> {
        ciborium::from_reader(self.payload.as_ref()).map_err(|source| ProtoError::Cbor {
            opcode: self.opcode.name(),
            source,
        })
    }

    /// Parses a payload that older or terser peers may legally omit entirely.
    pub fn parse_or_default<T: DeserializeOwned + Default>(&self) -> T {
        if self.payload.is_empty() {
            return T::default();
        }
        self.parse().unwrap_or_default()
    }

    pub fn encode_into(&self, dst: &mut BytesMut) {
        dst.reserve(HEADER_LEN + self.payload.len());
        dst.put_u8(MAGIC);
        dst.put_u8(VERSION);
        dst.put_u8(self.opcode as u8);
        dst.put_u8(0);
        dst.put_u32(self.stream.0);
        dst.put_u32(self.payload.len() as u32);
        dst.put_slice(&self.payload);
    }
}

/// A `DIFF` payload: an eight octet big-endian offset followed by object data (RFC R7).
#[derive(Clone, Debug)]
pub struct Diff {
    pub offset: u64,
    pub data: Bytes,
}

impl Diff {
    pub const OFFSET_LEN: usize = 8;

    pub fn decode(mut payload: Bytes) -> Result<Self, ProtoError> {
        if payload.len() < Self::OFFSET_LEN {
            return Err(ProtoError::ShortDiff(payload.len()));
        }
        let offset = payload.get_u64();
        Ok(Self {
            offset,
            data: payload,
        })
    }

    pub fn into_frame(self, stream: StreamId) -> Frame {
        let mut payload = BytesMut::with_capacity(Self::OFFSET_LEN + self.data.len());
        payload.put_u64(self.offset);
        payload.put_slice(&self.data);
        Frame::new(Opcode::Diff, stream, payload.freeze())
    }

    pub fn end(&self) -> u64 {
        self.offset.saturating_add(self.data.len() as u64)
    }
}
