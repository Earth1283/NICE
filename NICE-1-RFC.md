# NICE/1 — Implementation RFC

Companion to `NICE-1.md`. That document defines NICE/1. This one resolves the
questions it leaves open, so that two independent implementations can interoperate.

Nothing here contradicts `NICE-1.md`. Where the base specification says SHOULD or
MAY, this document states what `nicer` does and what a peer may rely on. Where the
base specification is silent, this document is normative for NICE/1 as implemented
by `nicer`, and is offered as errata.

Key words MUST, MUST NOT, SHOULD, SHOULD NOT, and MAY are used as in RFC 2119.

---

## R1. Frame header

The header is exactly 12 octets.

```text
 0               1               2               3
 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
+-------+-------+-------+-------+-------------------------------+
| Magic |  Ver  |Opcode | Rsvd  |          Stream ID            |
+-------+-------+-------+-------+-------------------------------+
|                           Length                              |
+---------------------------------------------------------------+
|                          Payload ...                          |
+---------------------------------------------------------------+
```

`Magic` is `0x69`. `Ver` is `0x01`. `Rsvd` MUST be zero.

A receiver observing a magic mismatch MUST close the connection without a reply.
The framing is not self-synchronising and no resynchronisation is defined; a peer
that has lost octet alignment cannot recover it.

A receiver observing a nonzero `Rsvd` MUST reply `CPP` and MUST close the
connection. `NICE-1.md` §5 permits `CPP` here; `nicer` always sends it.

A receiver observing an unknown `Ver` MUST reply `CPP` and close. Version
negotiation is not part of NICE/1.

An unknown `Opcode` MUST be answered with `CPP` on the frame's stream. The
connection SHOULD survive; unknown opcodes are how a future revision will look
from here, and closing on them would make NICE/2 undeployable.

## R2. Maximum frame size

`NICE-1.md` §5 requires allocation limits but names no value, leaving a sender no
way to know what a receiver will accept. NICE/1 resolves this by negotiation.

Every implementation MUST accept a payload of at least **65536** octets.

`HELLO` and `MERGED` MUST carry `max_frame_size`, the largest payload the sender
is willing to receive, in octets. It MUST be at least 65536. A peer MUST NOT send
a frame whose payload exceeds the value the other peer advertised.

A receiver MUST treat `Length` greater than its own advertised `max_frame_size`
as `CPP`, and MUST NOT allocate the buffer before rejecting it. The check happens
on the header, never after a partial read.

`nicer` advertises 1 MiB and refuses to be configured above 16 MiB.

Frames whose `Length` is legal but whose payload never arrives are governed by the
read timeout in R11, not by this section.

## R3. Stream identifiers

`NICE-1.md` §5 reserves stream `0` and says nothing about who allocates the rest,
which lets two peers open different operations under the same identifier.

The peer that sent `HELLO` is the **initiator**. The peer that answered `MERGED`
is the **responder**.

- Stream `0` is connection-level and belongs to neither peer.
- Initiator-opened streams MUST be odd.
- Responder-opened streams MUST be even and nonzero.

A stream is opened by the first frame that names it. Each peer MUST allocate
strictly increasing identifiers and MUST NOT reuse one within a connection.

A peer receiving a new stream of the wrong parity MUST reply `BROKE_USERSPACE` on
that stream and MUST close the connection.

A peer that exhausts its identifier space MUST NOT wrap. It MUST either open a new
connection or perform the R9 recovery exchange, which resets both allocators.

These opcodes are connection-level and MUST use stream `0`:

```text
HELLO  MERGED  TUX  SUBSURFACE  BITKEEPER  GIT  MONOTONE  NVIDIA
```

These are stream-scoped and MUST NOT use stream `0`:

```text
PULL_REQUEST  MERGE  FUCK_OFF  BIG_DIFF  DIFF  DONE  FSCK  CLEAN  CORRUPT  LKML
```

`CPP` and `SHUT_UP` are scoped by what they answer; see R7 and R8.

Violating the stream discipline for a given opcode is `BROKE_USERSPACE`.

## R4. Identity and TLS binding

`NICE-1.md` §4.1 requires Ed25519 authentication over TLS 1.3 without saying how
the key reaches the handshake. NICE/1 binds them as follows.

Each device holds one long-lived Ed25519 keypair, stored as PKCS#8 DER with owner-
only permissions.

For SECURE transport both peers present a self-signed X.509 certificate whose
`SubjectPublicKeyInfo` is that Ed25519 public key and whose signature is Ed25519
over the certificate itself. The server requests a client certificate and MUST
reject a connection that omits one; NICE/1 has no anonymous clients.

Certificate validation is entirely replaced:

- The chain MUST be exactly one certificate.
- Its self-signature MUST verify.
- No CA, name, or expiry check is performed. Hostnames are meaningless on a LAN
  with DHCP, and there is no issuer to trust.
- The identity is the `SubjectPublicKeyInfo`, taken verbatim as DER.

Possession of the private key is proven by TLS `CertificateVerify`. No additional
application-level challenge is defined or needed.

The **fingerprint** is `SHA-256(SubjectPublicKeyInfo DER)`, rendered lowercase hex.
Its short form for display is the first 8 octets, grouped in fours:

```text
9f3a1c04 b7e2d580
```

`HELLO` and `MERGED` MUST carry the sender's full fingerprint. Under SECURE it
MUST equal the fingerprint of the peer's TLS certificate; a mismatch is
`BROKE_USERSPACE` and MUST close the connection. The field is redundant under TLS
by design, so that a packet capture identifies both peers without the keys.

A certificate's Common Name MAY carry the device name. It is advisory, never an
identity, and MUST NOT be displayed in place of the fingerprint.

Under PLAINTEXT the fingerprint is a claim and nothing more. A PLAINTEXT peer MUST
NOT be paired, MUST NOT satisfy an existing pairing, and MUST be surfaced to the
user as unauthenticated.

## R5. Pairing

Pairing is trust on first use, confirmed by a human.

A first-time fingerprint MUST NOT be stored without user confirmation. The
confirming prompt MUST show the fingerprint. Implementations SHOULD show the short
form and make the full form obtainable.

A stored pairing binds fingerprint to a local record. The peer's IP address MUST
NOT be part of the identity and MUST NOT be compared against.

A fingerprint that does not match the one stored for a peer MUST fail the
connection with `NVIDIA` and MUST NOT be resolved by re-prompting the user. It is
reported as an identity change and is cleared only by an explicit unpair.

Pairing authenticates a device. It does not authorise a transfer; see R6.

## R6. Offers

`PULL_REQUEST` opens a stream and carries:

```text
kind        "file" | "clipboard"      required
size        octets, unsigned          required
mime        media type                optional
name        file name                 required for "file", forbidden for "clipboard"
preview     short excerpt             optional, "clipboard" only
```

`size` is exact. It is a promise, not a hint, and R7 enforces it.

`preview` MUST NOT exceed 256 octets and MUST be valid UTF-8. A receiver MUST
treat it as untrusted display text.

Exactly one of `MERGE`, `FUCK_OFF`, `SHUT_UP`, `BIG_DIFF` answers an offer, on the
offer's stream. Any other response is `BROKE_USERSPACE`.

`BIG_DIFF` occupies opcode `0x25` in the transfer block although it answers an
offer. `NICE-1.md` §7 forbids changing an assigned opcode's meaning, so the
number stays where it is.

`BIG_DIFF` SHOULD carry `max_size`, the largest object the receiver would have
accepted. It ends the stream. A sender MAY re-offer a smaller object on a new
stream, and MUST NOT re-offer the same object unchanged.

`FUCK_OFF` MAY carry `reason`. It ends the stream and is not an error.

A sender that can no longer deliver an object it has already offered MUST withdraw it by
sending `FUCK_OFF` on that stream, at any point up to `DONE`. The stream ends and the
receiver MUST discard whatever it has buffered. This is the only case in which the
offering peer sends `FUCK_OFF`, and it exists because `NICE-1.md` gives a sender no other
way to abandon a stream: the file it offered can be deleted, truncated, or become
unreadable between `PULL_REQUEST` and the last `DIFF`, and the alternative is a
deliberately incomplete transfer or a dropped connection.

A sender whose object changed size after it was offered MUST withdraw rather than send it,
because `DONE` requires the offered size exactly.

A receiver MUST NOT accept an offer without explicit user approval, except for
peers the user has marked as auto-accepting. That marking is per-peer and per-
kind, is never a default, and is independent of pairing.

`NICE-1.md` §17 is unambiguous:

```text
paired != permission to dump files onto my machine
```

## R7. Transfer

`DIFF` payloads are not CBOR. A `DIFF` payload is:

```text
+-------------------+---------------------------+
| offset, 8 octets  | data                      |
+-------------------+---------------------------+
```

`offset` is an unsigned big-endian 64-bit octet offset into the object. Eight
octets, not four: `size` is unbounded in CBOR, and a 32-bit offset would make
objects above 4 GiB describable in an offer and unrepresentable in transfer.

A `DIFF` payload shorter than 8 octets is `CPP`.

`nicer` sends chunks of 65536 octets, bounded by the peer's `max_frame_size`.

A conforming sender transmits `DIFF` frames in ascending contiguous order starting
at offset 0. A receiver MUST reject, as `BROKE_USERSPACE`, any `DIFF` that:

- arrives before `MERGE` was sent,
- arrives after `DONE`,
- does not begin where the previous `DIFF` on that stream ended,
- would carry the total beyond the offer's `size`.

Out-of-order and sparse writes are deliberately excluded. NICE/1 has no
retransmission, so the only thing a gap can mean is a broken sender.

`DONE` ends transmission. Its stream MUST have received `size` octets exactly;
short or long is `BROKE_USERSPACE`.

`DONE` carries no fields. Any payload is ignored.

## R8. Integrity and failure

`FSCK` follows `DONE` on the same stream and carries:

```text
algorithm    "BLAKE3"           required
digest       raw octets         required
total_size   octets, unsigned   required
```

`algorithm` is case-sensitive. `BLAKE3` MUST be supported. `SHA-256` MAY be. An
unrecognised algorithm is `CPP`.

The digest is over the whole object. `total_size` MUST equal the offer's `size`.

The receiver computes the digest itself and answers `CLEAN` or `CORRUPT`.

`CLEAN` commits the object. Only then may a received file be moved to its final
name, or clipboard content be applied. A receiver MUST complete that commit before it
sends `CLEAN`, not after: a receiver that verified the digest but could not store the
result MUST answer `CORRUPT`. From the sender's position the object did not arrive, which
is the part that matters, and `CLEAN` must never promise a file that failed to land.

`CORRUPT` ends the stream. The receiver MUST discard everything it buffered and
MUST NOT leave a partial object anywhere the user could mistake for a real one.

NICE/1 defines no retransmission and no resume. A sender MAY re-offer on a new
stream. `nicer` does not re-offer on its own: a fresh offer needs a fresh decision from
the receiving user, so retrying silently would spend someone else's attention.

Received files MUST be written inside a single configured directory. The sender's
`name` MUST be reduced to a bare filename with any path separator, drive letter,
`.`, `..`, control character, and NUL removed, and MUST NOT be permitted to
resolve outside that directory. Names colliding with an existing file MUST be
disambiguated locally, never overwritten. `NICE-1.md` §17 requires this; it is
restated because it is the requirement most likely to be skipped.

Every transfer MUST be written to a temporary file and renamed only after `CLEAN`.

## R9. Keepalive and recovery

A peer MAY send `TUX` after inactivity. A peer receiving `TUX` MUST answer
`SUBSURFACE`. Neither affects any stream.

`nicer` sends `TUX` after 30 s of inactivity and treats three unanswered
keepalives, or 90 s of silence, as unreachable.

`BITKEEPER` requests a fresh synchronisation point. A peer receiving it MUST
answer `GIT` or `MONOTONE`.

`GIT` resets the connection to its post-`MERGED` state. Both peers MUST abandon
every open stream, discard partial objects, and reset stream allocation to the
R3 initial state. Streams do not survive recovery.

`MONOTONE` declines. The requesting peer SHOULD close the connection, because a
peer that will not resynchronise cannot be reasoned about.

Recovery MUST NOT be used to escape a `BROKE_USERSPACE`. That error means state is
already untrustworthy, and the connection closes.

## R10. Chat

`NICE-1.md` §5 lists an LKML conversation among the things a stream carries, so
chat is not connection-level.

A conversation occupies one stream for the lifetime of the connection, opened by
whichever peer speaks first under R3 parity. Each `LKML` frame is one message.
Both peers use that stream once it exists. A peer that opens a second conversation stream
while one of its own is already established commits `BROKE_USERSPACE`.

Both peers can speak first. If each has opened a conversation stream before hearing the
other's, the initiator's stream wins: the responder MUST abandon its own and send
subsequent messages on the initiator's. Parity makes this decidable without negotiation,
since the initiator's identifiers are odd and the lower of the two competing streams is
always the initiator's. Messages already sent on the abandoned stream are still delivered;
nothing is retransmitted.

`LKML` carries:

```text
message_id   opaque, unique per sender     required
timestamp    milliseconds since UNIX epoch  required
text         UTF-8                          required
tags         map of string to string        optional
```

`text` MUST NOT exceed 4096 octets. Longer is `CPP`. A file shared in chat uses
`PULL_REQUEST` on its own stream, per `NICE-1.md` §13.

`timestamp` is the sender's clock and MUST NOT be trusted for ordering. Receivers
SHOULD order by arrival and MAY show a sender timestamp that disagrees.

`tags` carries the kernel-style metadata of `NICE-1.md` §13 (`Acked-by`,
`Reviewed-by`, `Tested-by`). It is display material with no protocol meaning.

The display name is the peer's current source IP, per `NICE-1.md` §3. The identity
remains the fingerprint. An implementation MUST be able to show which fingerprint
an IP resolved to, because on a LAN an IP is a suggestion.

## R11. Rate limiting and resource bounds

`SHUT_UP` carries:

```text
retry_after_ms   unsigned                         required
scope            "stream" | "kind" | "connection"  optional, default "connection"
kind             opcode name, when scope is "kind" optional
```

Scope is added because `NICE-1.md` §14 rate-limits "offers, chat messages, or
other requests" without saying how much a peer must stop doing. Without it, a peer
throttled for chat cannot tell whether file transfer is still permitted.

A peer receiving `SHUT_UP` MUST stop initiating operations in scope until the
interval expires. In-flight transfers continue; `SHUT_UP` never interrupts an
accepted object.

`retry_after_ms` above 3600000 SHOULD be treated as 3600000. A peer that keeps
initiating in scope MAY be answered with `NVIDIA` and disconnected.

`SHUT_UP` answering a specific frame uses that frame's stream; otherwise stream 0.

A peer MUST bound, per connection: concurrent streams (`nicer`: 64), unanswered
offers (`nicer`: 8), and total buffered transfer state. Exceeding a bound is
`SHUT_UP`, not a disconnect.

A peer MUST bound concurrent connections per source address (`nicer`: 4) and MUST
apply a read timeout to any frame whose header has arrived (`nicer`: 30 s) and to
a connection that has not completed `HELLO`/`MERGED` (`nicer`: 10 s).

## R12. Error opcodes

`CPP`, `BROKE_USERSPACE`, `NVIDIA`, and `MONOTONE` MAY carry:

```text
reason   short UTF-8 diagnostic, at most 256 octets   optional
```

`reason` is for humans. It MUST NOT be parsed and MUST NOT change behaviour.

| Opcode | Meaning | Connection |
|---|---|---|
| `CPP` | frame could not be processed | survives, unless framing itself is lost |
| `BROKE_USERSPACE` | protocol-state guarantee violated | MUST close |
| `NVIDIA` | refusal at connection level | MUST close |
| `MONOTONE` | recovery declined | SHOULD close |
| `FUCK_OFF` | operation rejected | survives, not an error |
| `SHUT_UP` | rate limited | survives |

A peer MUST NOT answer an error frame with another error frame. `nicer` logs and
drops.

An implementation SHOULD distinguish a frame it could not parse (`CPP`) from a
frame it parsed and found illegal in context (`BROKE_USERSPACE`). `NICE-1.md` §19
predicts this will be done badly. It is at least worth trying.

## R13. Discovery

Service `_nice._tcp.local`, default port 6969.

TXT records:

```text
version=1                 required
device=<name>             required, UTF-8, at most 63 octets
secure=yes|no             required
fp=<hex fingerprint>      recommended
```

`fp` lets a user recognise a known device before connecting, and lets an unknown
one be flagged before a handshake. It is not authentication: TXT records are
unauthenticated and trivially forged. It MUST be confirmed against the TLS
certificate before it means anything.

`device` is untrusted remote input. It MUST be truncated and stripped of control
characters before display, and MUST NOT be used to construct a path.

Discovery MUST NOT imply trust, MUST NOT trigger a connection, and MUST NOT
trigger a pairing prompt.

## R14. Transport selection

SECURE is the default. PLAINTEXT MUST require explicit configuration and MUST NOT
be reachable by a flag alone or by falling back from a failed TLS handshake.

A SECURE listener MUST NOT accept a PLAINTEXT connection on the same port.

A peer configured for PLAINTEXT MUST warn, per `NICE-1.md` §4.2:

```text
WARNING: YOU ASKED WIRESHARK TO READ YOUR SHITPOSTS
```

TLS key logging MUST be off unless explicitly enabled, MUST warn when enabled, and
MUST write only to a path the user named.

## R15. Summary of resolutions

| # | `NICE-1.md` gap | Resolution |
|---|---|---|
| R1 | resync after bad magic | none; close |
| R2 | "reasonable allocation limits" | negotiated `max_frame_size`, floor 65536 |
| R3 | stream allocation | initiator odd, responder even, monotonic |
| R3 | opcode-to-stream mapping | fixed table |
| R4 | Ed25519 in TLS | self-signed cert, SPKI is the identity, pinned |
| R4 | identity in PLAINTEXT | unauthenticated claim, cannot pair |
| R6 | `BIG_DIFF` semantics | offer response at `0x25`, carries `max_size` |
| R6 | withdrawing an offered object | sender sends `FUCK_OFF` on that stream |
| R7 | `DIFF` offset width | u64 big-endian |
| R7 | `DIFF` ordering | contiguous ascending, gaps are errors |
| R8 | recovery after `CORRUPT` | no resume; re-offer on a new stream |
| R8 | commit fails after a good digest | answer `CORRUPT`; commit precedes `CLEAN` |
| R9 | what `GIT` resets | all streams abandoned, allocators reset |
| R10 | stream for `LKML` | one conversation stream, not stream 0 |
| R10 | both peers open a conversation | the initiator's stream wins |
| R11 | scope of `SHUT_UP` | explicit `scope` field |
| R12 | error payloads | optional `reason`, advisory only |
| R13 | fingerprint in discovery | `fp` TXT record, advisory only |
