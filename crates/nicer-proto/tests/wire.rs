use bytes::{Bytes, BytesMut};
use nicer_proto::payload::{OfferKind, PullRequest};
use nicer_proto::{
    Diff, Frame, Incoming, NiceCodec, Opcode, ProtoError, Role, StreamAllocator, StreamId,
    HEADER_LEN, MAGIC,
};
use tokio_util::codec::{Decoder, Encoder};

fn codec() -> NiceCodec {
    NiceCodec::new(65536)
}

fn roundtrip(frame: Frame) -> Incoming {
    let mut buffer = BytesMut::new();
    codec().encode(frame, &mut buffer).unwrap();
    codec().decode(&mut buffer).unwrap().unwrap()
}

#[test]
fn a_frame_survives_the_wire() {
    let offer = PullRequest {
        kind: OfferKind::File,
        size: 812944,
        mime: Some("image/png".into()),
        name: Some("extremely_important_cat.png".into()),
        preview: None,
    };
    let frame = Frame::cbor(Opcode::PullRequest, StreamId(9), &offer).unwrap();

    let Incoming::Frame(decoded) = roundtrip(frame) else {
        panic!("a defined opcode decoded as unknown");
    };
    assert_eq!(decoded.opcode, Opcode::PullRequest);
    assert_eq!(decoded.stream, StreamId(9));

    let parsed: PullRequest = decoded.parse().unwrap();
    assert_eq!(parsed.name.as_deref(), Some("extremely_important_cat.png"));
    assert_eq!(parsed.size, 812944);
}

#[test]
fn the_header_is_twelve_octets_of_the_documented_shape() {
    let mut buffer = BytesMut::new();
    codec()
        .encode(
            Frame::new(
                Opcode::Diff,
                StreamId(0x01020304),
                Bytes::from_static(b"xy"),
            ),
            &mut buffer,
        )
        .unwrap();

    assert_eq!(buffer.len(), HEADER_LEN + 2);
    assert_eq!(buffer[0], MAGIC);
    assert_eq!(buffer[1], 0x01);
    assert_eq!(buffer[2], Opcode::Diff as u8);
    assert_eq!(buffer[3], 0x00);
    assert_eq!(&buffer[4..8], &[0x01, 0x02, 0x03, 0x04]);
    assert_eq!(&buffer[8..12], &[0x00, 0x00, 0x00, 0x02]);
}

/// RFC R1: an opcode this revision does not define must stay recoverable, so that NICE/2
/// is deployable against NICE/1 peers.
#[test]
fn an_undefined_opcode_is_preserved_rather_than_fatal() {
    let mut buffer = BytesMut::new();
    buffer.extend_from_slice(&[MAGIC, 0x01, 0x5A, 0x00, 0, 0, 0, 7, 0, 0, 0, 1, 0xFF]);

    match codec().decode(&mut buffer).unwrap().unwrap() {
        Incoming::Unknown { opcode, stream, .. } => {
            assert_eq!(opcode, 0x5A);
            assert_eq!(stream, StreamId(7));
        }
        Incoming::Frame(frame) => panic!("0x5A decoded as {}", frame.opcode),
    }
}

#[test]
fn a_nonzero_reserved_octet_is_rejected() {
    let mut buffer = BytesMut::new();
    buffer.extend_from_slice(&[MAGIC, 0x01, 0x03, 0x01, 0, 0, 0, 0, 0, 0, 0, 0]);
    assert!(matches!(
        codec().decode(&mut buffer),
        Err(ProtoError::ReservedNotZero(0x01))
    ));
}

#[test]
fn bad_magic_loses_framing() {
    let mut buffer = BytesMut::new();
    buffer.extend_from_slice(&[0x68, 0x01, 0x03, 0x00, 0, 0, 0, 0, 0, 0, 0, 0]);
    let error = codec().decode(&mut buffer).unwrap_err();
    assert!(matches!(error, ProtoError::BadMagic(0x68)));
    assert!(error.is_framing_loss());
}

/// RFC R2: the length is judged from the header, before any payload buffer is reserved.
#[test]
fn an_oversized_length_is_refused_from_the_header_alone() {
    let mut buffer = BytesMut::new();
    buffer.extend_from_slice(&[MAGIC, 0x01, 0x20, 0x00, 0, 0, 0, 1, 0xFF, 0xFF, 0xFF, 0xFF]);
    let error = codec().decode(&mut buffer).unwrap_err();
    assert!(matches!(
        error,
        ProtoError::FrameTooLarge {
            len: 0xFFFF_FFFF,
            limit: 65536
        }
    ));
    assert_eq!(buffer.len(), HEADER_LEN, "the header must not be consumed");
}

#[test]
fn a_partial_frame_waits_for_the_rest() {
    let mut buffer = BytesMut::new();
    codec()
        .encode(
            Frame::new(Opcode::Lkml, StreamId(1), Bytes::from_static(b"hello")),
            &mut buffer,
        )
        .unwrap();
    let tail = buffer.split_off(HEADER_LEN + 2);

    assert!(codec().decode(&mut buffer).unwrap().is_none());
    buffer.extend_from_slice(&tail);
    assert!(codec().decode(&mut buffer).unwrap().is_some());
}

#[test]
fn a_frame_over_the_peers_limit_is_never_sent() {
    let mut codec = NiceCodec::new(65536);
    codec.set_peer_limit(64);
    let mut buffer = BytesMut::new();
    let oversized = Frame::new(Opcode::Diff, StreamId(1), Bytes::from(vec![0u8; 128]));
    assert!(matches!(
        codec.encode(oversized, &mut buffer),
        Err(ProtoError::FrameTooLarge {
            len: 128,
            limit: 64
        })
    ));
}

/// RFC R7: the offset is eight octets, so an object above 4 GiB stays representable.
#[test]
fn a_diff_carries_a_sixty_four_bit_offset() {
    let offset = 5_000_000_000;
    let frame = Diff {
        offset,
        data: Bytes::from_static(b"payload"),
    }
    .into_frame(StreamId(3));

    assert_eq!(frame.payload.len(), 8 + 7);
    let decoded = Diff::decode(frame.payload).unwrap();
    assert_eq!(decoded.offset, offset);
    assert_eq!(decoded.data.as_ref(), b"payload");
    assert_eq!(decoded.end(), offset + 7);
}

#[test]
fn a_diff_without_room_for_its_offset_is_rejected() {
    assert!(matches!(
        Diff::decode(Bytes::from_static(&[0, 0, 0, 1])),
        Err(ProtoError::ShortDiff(4))
    ));
}

/// RFC R3: initiator streams are odd, responder streams are even, and neither wraps.
#[test]
fn stream_identifiers_are_partitioned_by_role() {
    let mut initiator = StreamAllocator::new(Role::Initiator);
    let mut responder = StreamAllocator::new(Role::Responder);

    assert_eq!(initiator.allocate(), Some(StreamId(1)));
    assert_eq!(initiator.allocate(), Some(StreamId(3)));
    assert_eq!(responder.allocate(), Some(StreamId(2)));
    assert_eq!(responder.allocate(), Some(StreamId(4)));

    assert!(Role::Initiator.owns(StreamId(7)));
    assert!(!Role::Initiator.owns(StreamId(8)));
    assert!(Role::Responder.owns(StreamId(8)));
    assert!(!Role::Responder.owns(StreamId(0)));
    assert!(!Role::Initiator.owns(StreamId(0)));

    initiator.reset();
    assert_eq!(initiator.allocate(), Some(StreamId(1)));
}

#[test]
fn peer_streams_reject_reuse_and_wrong_parity() {
    let mut seen = nicer_proto::PeerStreams::new(Role::Initiator);

    assert!(seen.accept_new(StreamId(2)).is_ok());
    assert!(seen.accept_new(StreamId(4)).is_ok());
    assert!(seen.accept_new(StreamId(4)).is_err());
    assert!(seen.accept_new(StreamId(2)).is_err());
    assert!(seen.accept_new(StreamId(5)).is_err());
    assert!(seen.is_stale(StreamId(2)));
}

#[test]
fn opcode_names_and_scopes_match_the_registry() {
    assert_eq!(Opcode::from_u8(0x7D), Some(Opcode::BrokeUserspace));
    assert_eq!(Opcode::BrokeUserspace.name(), "BROKE_USERSPACE");
    assert_eq!(Opcode::from_u8(0x99), None);

    for opcode in Opcode::ALL {
        assert_eq!(Opcode::from_u8(opcode as u8), Some(opcode));
    }

    use nicer_proto::Scope;
    assert_eq!(Opcode::Hello.scope(), Scope::Connection);
    assert_eq!(Opcode::Lkml.scope(), Scope::Stream);
    assert_eq!(Opcode::ShutUp.scope(), Scope::Either);
}
