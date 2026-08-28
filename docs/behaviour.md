# Daemon behaviour

What the daemon does, in the order it does it. Section references in the form R3 point at
[`NICE-1-RFC.md`](../NICE-1-RFC.md); references like §12 point at [`NICE-1.md`](../NICE-1.md).

## Startup

1. Read `config.toml`, apply command-line overrides, validate. Invalid configuration is
   fatal; the daemon does not start with a configuration it cannot honour.
2. Load `identity.key`, or generate an Ed25519 keypair on first run and write it with
   owner-only permissions. Build the TLS certificate from it. The certificate is rebuilt
   at every start, so renaming the device does not change the identity.
3. Load `peers.json`.
4. Bind the TCP listener. Bind the control endpoint.
5. Start mDNS, unless discovery is off.
6. Emit `listening`.

If PLAINTEXT is configured, the warning from §4.2 is printed to stderr before anything
else happens. If TLS key logging is configured, a warning is printed too.

## Discovery

The daemon registers `_nice._tcp.local` with TXT records `version`, `device`, `secure`,
and `fp`, and browses for the same service. Its own advertisement is filtered out.

Discovery emits `peer_discovered` and `peer_lost`. It does nothing else. It does not
connect, does not prompt, and does not pair. The `fp` record is a hint that lets a user
recognise a device before connecting; it is unauthenticated and means nothing until the
TLS certificate confirms it.

`device` and the instance name are remote input. Both are stripped of control characters
and truncated before they appear in an event.

## Connections

An inbound connection is refused before the handshake if the source address already holds
`limits.max_connections_per_address` connections.

Both directions then run the same sequence, under `timeouts.handshake_secs`:

1. TLS, if SECURE. The peer must present exactly one self-signed certificate. When
   dialling a peer whose identity is already stored for that address, that identity is
   pinned and a mismatch fails the handshake (R5).
2. `HELLO` from the initiator, `MERGED` from the responder (§8). Both carry the protocol
   version, device name, fingerprint, transport mode, `max_frame_size`, and capabilities.
3. The version must be 1. The peer's `max_frame_size` must be at least 65536. Under
   SECURE, the fingerprint in the greeting must equal the one the certificate proved; a
   mismatch fails the connection (R4).
4. The codec is told the peer's limit. Nothing larger is ever sent.

On success the node emits `connected`. If the peer is authenticated but not paired, it
also emits `pairing_required`. The connection is usable either way — being unpaired
restricts what can be auto-accepted, not whether the peer can talk.

The role fixed here lasts for the connection: the peer that sent `HELLO` is the initiator
and owns odd stream identifiers; the responder owns even ones (R3).

## Pairing

Pairing is trust on first use, confirmed by a human, and is driven entirely by the
frontend. The daemon never pairs on its own.

`pair` stores the fingerprint, a device name, and the time. `unpair` removes it. A stored
peer records the address it was last seen at, which is what makes pinning possible on the
next outbound connection to that address.

Pairing takes effect immediately on connections that are already open. Whether a peer is
paired is read from the store at the moment it matters, not cached when the connection
was made.

An identity that no longer matches a stored one is reported as `identity_changed` and
fails the connection. It is not re-offered as a fresh pairing prompt, and is cleared only
by an explicit `unpair`.

## Offers

Clipboard and file transfers use the same mechanism (§10). The sender opens a stream with
`PULL_REQUEST`; exactly one of `MERGE`, `FUCK_OFF`, `BIG_DIFF`, or `SHUT_UP` answers it.

Receiving an offer, in order:

1. Parse and validate. A file offer must carry a name; a clipboard offer must not. A
   `preview` over 256 octets is rejected. Failures are `CPP`.
2. Check stream parity and that the identifier increases. Failures are `BROKE_USERSPACE`.
3. Check the stream count and the number of offers already awaiting a verdict. Over
   either limit, answer `SHUT_UP` scoped to `PULL_REQUEST`.
4. Check the offer rate. Over budget, answer `SHUT_UP` scoped to `PULL_REQUEST`.
5. Check the size against `limits.max_object_size`. Over it, answer `BIG_DIFF` carrying
   that limit.
6. Emit `offer`. If the peer is paired and marked auto-accepting for that kind, answer
   `MERGE` immediately; the event still fires, with `auto_accepted` true.

An offer nobody answers within 300 seconds is refused automatically, so a stream cannot be
held open by an unanswered prompt.

Offers are never accepted by default. Auto-acceptance is per peer and per kind, is set
only by an explicit command, and requires pairing.

## Sending

After `MERGE` the sender opens the object and checks that its size still matches what was
offered. If it does not, or if the object cannot be opened, the sender withdraws with
`FUCK_OFF` on that stream (R6) rather than transmitting something that cannot satisfy
`DONE`.

The object is then read in chunks of `limits.chunk_size`, further bounded by the peer's
`max_frame_size` less the eight octets of `DIFF` offset. Each chunk is hashed with BLAKE3
as it is read, so `FSCK` needs no second pass.

`DIFF` frames are contiguous and ascending from offset 0. The sender concludes with
`DONE`, then `FSCK` carrying the algorithm, the digest, and the total size, then waits for
`CLEAN` or `CORRUPT`.

`transfer_progress` is emitted every mebibyte, not every frame.

## Receiving

A file is written to a temporary `.part` file in the download directory. Clipboard
content is accumulated in memory.

Each `DIFF` must continue exactly where the previous one ended and must not carry the
total past the offered size. A gap, an overlap, or an overrun is `BROKE_USERSPACE`: NICE/1
has no retransmission, so the only thing a gap can mean is a broken sender.

`DONE` requires the received length to equal the offered size exactly.

`FSCK` must name a supported algorithm — BLAKE3 always, SHA-256 optionally — and its
`total_size` must match the offer. The digest is computed locally over the whole object.
Because the algorithm is not known until `FSCK` arrives, both supported digests are
computed as the object streams past.

If the digest matches, the object is committed first and `CLEAN` is sent only after the
commit succeeds. A verified object that cannot be stored is answered `CORRUPT` (R8): from
the sender's position it did not arrive, and `CLEAN` must not promise a file that failed
to land.

If the digest does not match, `CORRUPT` is sent and the temporary file is removed.

A committed file is renamed into place under a name derived from the sender's, reduced to
a bare filename and disambiguated if it collides. See [security.md](security.md).

## Chat

An `LKML` conversation occupies one stream, opened by whichever peer speaks first (R10).
If both peers open one before hearing the other, the initiator's stream wins and the
responder switches to it. A peer that opens a second conversation stream while one of its
own is established commits `BROKE_USERSPACE`.

Messages carry an identifier, a timestamp, text, and optional tags. Text over 4096 octets
is `CPP`. The sender's timestamp is passed through and is not trusted for ordering.

The `from` field of a `chat` event is the peer's current source address, which is the
display name §3 asks for. The `fingerprint` field is the identity. Both are given, because
on a LAN an address is a suggestion.

Files shared in a conversation use `PULL_REQUEST` on their own stream. There is no
attachment mechanism inside `LKML`.

## Keepalive and timeouts

After `timeouts.keepalive_secs` of inactivity the daemon sends `TUX`. A received `TUX` is
answered `SUBSURFACE` immediately. Three unanswered keepalives, or
`timeouts.idle_secs` of total silence, close the connection. Neither message disturbs a
transfer.

A connection that has not completed the handshake within `timeouts.handshake_secs` is
dropped.

## Rate limiting

Three budgets are measured per connection: offers per minute, chat messages per minute,
and frames per second. Each is a leaky bucket that reports how long the peer must wait,
which is what `retry_after_ms` carries.

Exceeding the offer or chat budget answers `SHUT_UP` on the offending stream, scoped to
that opcode, and the request is dropped. Exceeding the frame budget answers `SHUT_UP`
scoped to the connection, at most once every five seconds, and then delays processing —
frames are never dropped, because dropping one would corrupt stream state.

A `SHUT_UP` received is recorded, and the daemon stops initiating operations in scope
until the interval expires. `retry_after_ms` above one hour is treated as one hour. An
in-flight transfer is never interrupted by `SHUT_UP`.

A peer that provokes more than five rate limits on one connection is sent `NVIDIA` and
disconnected (§14).

## Recovery

`BITKEEPER` requests a fresh synchronisation point. The daemon answers `GIT` and resets:
every open stream is abandoned, every partial object is discarded, both stream allocators
return to their initial state, and the conversation stream is forgotten (R9). `resync`
performs the same reset locally and then asks the peer to do it.

`MONOTONE` from a peer closes the connection: a peer that will not resynchronise cannot be
reasoned about.

Recovery cannot be used to escape a `BROKE_USERSPACE`. That error means the state is
already untrustworthy, and the connection closes.

## Unknown opcodes and future revisions

An opcode this revision does not define is answered `CPP` on its stream, and the
connection continues. Closing on unknown opcodes would make a future NICE/2 undeployable
against NICE/1 peers. Unknown CBOR fields are ignored for the same reason (§6).

## Shutdown

`SIGINT` stops the daemon. mDNS deregisters, the control endpoint is removed, and open
connections close.

A transfer in flight when the daemon stops leaves its temporary file behind. Streams
abandoned while the daemon is running — a refused offer, a corrupt object, a closed
connection, a `GIT` reset — do clean up after themselves, but a process that goes away
does not get to run that code. Temporary files are named `.nicer-<millis>-<pid>.part` in
the download directory and are safe to delete once no daemon is running.
