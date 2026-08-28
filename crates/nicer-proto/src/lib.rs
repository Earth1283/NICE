//! The NICE/1 wire format, as specified by `NICE-1.md` and resolved by `NICE-1-RFC.md`.
//!
//! This crate encodes and decodes frames. It holds no connection state and enforces no
//! policy; both belong to `nicer`.

pub mod codec;
pub mod error;
pub mod frame;
pub mod opcode;
pub mod payload;
pub mod stream;

pub use codec::{Incoming, NiceCodec};
pub use error::{Fault, ProtoError};
pub use frame::{
    Diff, Frame, DEFAULT_MAX_FRAME_SIZE, HARD_MAX_FRAME_SIZE, HEADER_LEN, MAGIC,
    MIN_MAX_FRAME_SIZE, VERSION,
};
pub use opcode::{Opcode, Scope};
pub use stream::{PeerStreams, Role, StreamAllocator, StreamId, StreamViolation, CONNECTION};

pub const PROTOCOL_VERSION: u8 = VERSION;
pub const DEFAULT_PORT: u16 = 6969;
pub const SERVICE_TYPE: &str = "_nice._tcp.local.";
