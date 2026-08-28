# Implementing a NICE/1 peer

This document is for a client speaking NICE/1 to `nicer` over the network, rather than a
frontend driving it over the control socket. [`NICE-1.md`](../NICE-1.md) is the protocol
and [`NICE-1-RFC.md`](../NICE-1-RFC.md) resolves what it leaves open; this describes what
`nicer` specifically requires, sends, and refuses.

§19 warns that a custom client should expect a significant number of `BROKE_USERSPACE`,
`CPP`, and `NVIDIA` responses. What follows is an attempt to make that avoidable.

## Minimum viable peer

To exchange one file with `nicer` you need:

- The 12-octet frame header, big-endian.
- CBOR for control payloads.
- `HELLO`/`MERGED`.
- `PULL_REQUEST`, `MERGE`, `FUCK_OFF`.
- `DIFF`, `DONE`, `FSCK`, `CLEAN`, `CORRUPT`.
- BLAKE3.
- `TUX`/`SUBSURFACE`, or a connection that talks often enough never to go idle.

TLS is required unless `nicer` is configured for PLAINTEXT, which it is not by default.
`LKML`, `BITKEEPER`/`GIT`, and `SHUT_UP` can be received and ignored to begin with, though
ignoring `SHUT_UP` eventually earns `NVIDIA`.

## Transport

`nicer` listens on TCP 6969 by default and advertises `_nice._tcp.local` with TXT records
`version=1`, `device`, `secure`, and `fp`. The `fp` record is the peer's fingerprint and
is unauthenticated; treat it as a hint for recognising a device, never as proof.

A SECURE listener accepts only TLS. It does not fall back to PLAINTEXT, and there is no
in-band upgrade. TLS 1.3 only.

Both sides present exactly one self-signed X.509 certificate whose `SubjectPublicKeyInfo`
is an Ed25519 public key, signed by that key. `nicer` requests a client certificate and
refuses a connection without one. It performs no CA, hostname, or expiry check — the only
questions are whether the chain has one certificate, whether it is self-signed, and what
its `SubjectPublicKeyInfo` is. A client may send any `server_name`; `nicer` sends
`nice.invalid` and ignores whatever it receives.

The identity is `SHA-256(SubjectPublicKeyInfo DER)`, rendered as 64 lowercase hex
characters.

## Handshake

The initiator sends `HELLO` on stream 0 and waits for `MERGED`. `nicer` accepts nothing
else first, and closes on anything that is not the frame it expects.

```
HELLO / MERGED {
    version: 1,
    device: "thinkpad",
    fingerprint: "9f3a…",          64 lowercase hex characters
    transport: "secure",           or "plaintext"
    max_frame_size: 1048576,       at least 65536
    capabilities: ["file", "clipboard", "chat", "blake3"]
}
```

`nicer` fails the connection if the version is not 1, if `max_frame_size` is below 65536,
or if — under TLS — `fingerprint` is not the one the certificate proved. That last field
is redundant under TLS on purpose: it makes a packet capture identify both peers without
the keys.

`capabilities` is advisory. `nicer` does not gate behaviour on it, and unknown entries are
ignored.

## Framing

```
+--------+---------+--------+----------+-----------+---------+
| 0x69   | 0x01    | opcode | 0x00     | stream    | length  |
| 1 byte | 1 byte  | 1 byte | 1 byte   | 4 bytes   | 4 bytes |
+--------+---------+--------+----------+-----------+---------+
```

All integers unsigned big-endian. The reserved octet must be zero; a nonzero one earns
`CPP` and a close.

`length` is checked against the receiver's advertised `max_frame_size` from the header
alone, before any buffer is reserved. Exceeding it loses framing, so `nicer` closes without
replying — a peer that sees the connection drop with no error frame should suspect an
oversized length or bad magic.

`nicer` advertises 1 MiB by default and never sends a frame larger than whatever the peer
advertised.

## Streams

Stream 0 is connection-level. The peer that sent `HELLO` owns odd identifiers; the peer
that sent `MERGED` owns even ones. Identifiers must strictly increase and are never
reused within a connection.

These must use stream 0:

```
HELLO  MERGED  TUX  SUBSURFACE  BITKEEPER  GIT  MONOTONE  NVIDIA
```

These must not:

```
PULL_REQUEST  MERGE  FUCK_OFF  BIG_DIFF  DIFF  DONE  FSCK  CLEAN  CORRUPT  LKML
```

`CPP` and `SHUT_UP` go wherever what they answer went.

Using the wrong stream for an opcode, or opening a stream of the wrong parity, is
`BROKE_USERSPACE`.

## A conformant exchange

```
→ HELLO          stream 0
← MERGED         stream 0

→ PULL_REQUEST   stream 1   {kind, size, name, mime}
← MERGE          stream 1

→ DIFF           stream 1   offset 0
→ DIFF           stream 1   offset 65536
→ DONE           stream 1
→ FSCK           stream 1   {algorithm: "BLAKE3", digest, total_size}
← CLEAN          stream 1
```

`nicer` will not answer `MERGE` until a user accepts, unless that peer is paired and marked
auto-accepting. There is no timeout a sender can rely on other than its own; `nicer`
refuses an unanswered offer after five minutes.

## What earns which error

`CPP` — the frame could not be processed. The connection survives.

- A payload that is not valid CBOR, or does not match the opcode's shape.
- A `PULL_REQUEST` whose `kind` is `file` with no `name`, or `clipboard` with one.
- A `preview` over 256 octets, or `LKML` `text` over 4096.
- A `DIFF` payload shorter than its eight-octet offset.
- An `FSCK` naming an algorithm other than `BLAKE3` or `SHA-256`.
- An opcode this revision does not define. `nicer` answers `CPP` and keeps going, so that
  a future revision remains deployable against it.
- A nonzero reserved octet, or a version other than 1 — these also close.

`BROKE_USERSPACE` — a protocol-state guarantee was violated. The connection closes.

- `DIFF` before `MERGE`, or after `DONE`.
- A `DIFF` that does not begin where the previous one ended, or that carries the object
  past the offered size.
- `DONE` when the received length is not exactly the offered size.
- `FSCK` before `DONE`, or an `FSCK` whose `total_size` disagrees with the offer.
- `CLEAN` or `CORRUPT` on a stream not awaiting verification.
- `MERGE`, `FUCK_OFF`, or `BIG_DIFF` on a stream carrying no offer.
- A stream of the wrong parity, a reused identifier, or an identifier that moves backwards.
- A connection-level opcode on a stream, or a stream-scoped opcode on stream 0.
- `HELLO` or `MERGED` after the handshake.
- A second `LKML` conversation stream.

`NVIDIA` — refusal at the connection level. The connection closes.

- More than five rate limits provoked on one connection.

`SHUT_UP` — a budget was exceeded. The connection survives.

- More than 30 offers per minute, or 120 chat messages per minute, by default.
- More than 512 frames per second, by default. `nicer` sends this at most once every five
  seconds and then slows its reads; it does not drop frames.
- More than 64 open streams, or more than 8 offers awaiting a verdict.

`FUCK_OFF` is not an error. It is a refused offer, or an offering peer withdrawing.

`nicer` never answers an error frame with another error frame.

## Transfer details

`DIFF` payloads are not CBOR:

```
+--------------------+--------------------+
| offset, 8 octets   | data               |
+--------------------+--------------------+
```

The offset is a big-endian unsigned 64-bit octet offset into the object. Frames must be
contiguous and ascending from 0. There is no retransmission and no resume, so a gap cannot
be repaired and is treated as a broken sender.

`nicer` sends 65536-octet chunks by default, bounded by the peer's `max_frame_size` less
the eight octets of offset.

`FSCK` carries `algorithm`, `digest` as a CBOR byte string, and `total_size`. Use a CBOR
byte string, not an array of integers. `BLAKE3` must be supported; `SHA-256` is optional
and `nicer` accepts it.

A sender that can no longer deliver an object it offered sends `FUCK_OFF` on that stream to
withdraw it. This is the only case in which the offering peer sends `FUCK_OFF`, and it
exists because nothing else in NICE/1 lets a sender abandon a stream.

`nicer` answers `CORRUPT` when the digest does not match, and also when the digest matched
but it could not store the result — from a sender's position those are the same outcome. A
sender may re-offer on a new stream; `nicer` gives up after two attempts.

## Chat

One conversation stream per connection, opened by whichever peer speaks first. If both
open one before hearing the other, the initiator's wins and the responder must move to it.
Opening a second while one of your own is established is `BROKE_USERSPACE`.

```
LKML {
    message_id: "…",              unique per sender
    timestamp: 1787920057315,     milliseconds since the epoch
    text: "this meme sucks",      at most 4096 octets
    tags: {"Acked-by": "…"}       optional, display only
}
```

`nicer` does not trust `timestamp` for ordering, and neither should you. Files shared in a
conversation use `PULL_REQUEST` on their own stream.

## Keepalive and recovery

`nicer` sends `TUX` after 30 seconds of inactivity and expects `SUBSURFACE`. Three
unanswered, or 90 seconds of total silence, and it closes. Answer `TUX` promptly; neither
message affects transfer state.

`BITKEEPER` asks for a fresh synchronisation point. `nicer` answers `GIT`, abandons every
open stream, discards partial objects, and resets both allocators to their initial state.
Send `MONOTONE` if you will not; `nicer` will close the connection, because a peer that
will not resynchronise cannot be reasoned about.

Recovery does not rescue a `BROKE_USERSPACE`. That connection is already closing.

## Things that will bite you

- **Stream parity.** Half of all `BROKE_USERSPACE` responses from a new client are an
  initiator using even identifiers because it started counting at 0 or 2.
- **`digest` as an array.** CBOR distinguishes byte strings from arrays of integers. Many
  libraries serialise a byte slice as the latter by default.
- **Offsets in the first `DIFF`.** It must be 0, and the next must be exactly the previous
  offset plus the previous length. Not the chunk index, and not a running frame count.
- **`size` as a promise.** It is exact, and `DONE` is checked against it. If the object can
  change under you, hash it first or withdraw.
- **Sending `DIFF` on optimism.** Waiting for `MERGE` is not a formality; a human is
  usually reading a prompt.
- **Assuming a reply per frame.** `MERGE`, `DIFF`, and `DONE` are not individually
  acknowledged. Only `PULL_REQUEST` and `FSCK` have answers.
- **Certificate expiry.** `nicer` does not check it, but do not assume the reverse of
  anything else that speaks NICE/1.

## Developing against it

Configure `nicer` for PLAINTEXT to read frames directly. It requires two acknowledgements,
because it is not a mode to end up in by accident:

```toml
[transport]
mode = "plaintext"
i_know_wireshark_can_read_this = true
```

```sh
nicer run --plaintext
```

The `--plaintext` flag alone is refused; the configuration file must agree. The daemon
prints the warning from §4.2 when it starts in this mode.

To read encrypted traffic instead, set `keylog` to a path and point Wireshark's TLS
pre-master secret log at it. The daemon warns when this is on.

Opcodes are numerically stable and symbolic names are in `nicer-proto`'s registry, which
is the place to generate a dissector's value string from.
