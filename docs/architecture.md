# Architecture

## Crates

```
crates/nicer-proto    the wire format
crates/nicer          the daemon, as a library and the `nicer` binary
```

`nicer-proto` encodes and decodes frames. It holds no connection state and enforces no
policy: it will happily encode a `DIFF` on stream 0, because deciding that this is illegal
is the session's job. The split exists so that a dissector, a fuzzer, or a second client
can depend on the wire format without dragging in TLS, mDNS, and a clipboard.

`nicer` is both a library and a binary. The binary is a thin wrapper: argument parsing,
logging, and wiring `Node` to the control interface.

## Modules

| Module | Responsibility |
|---|---|
| `proto::frame` | Header layout, `Frame`, `Diff` payload encoding |
| `proto::codec` | `NiceCodec`, the tokio `Decoder`/`Encoder` pair, frame size enforcement |
| `proto::opcode` | The opcode registry, symbolic names, stream scope |
| `proto::payload` | CBOR payload types and their field limits |
| `proto::stream` | `StreamId`, role parity, allocation, peer stream validation |
| `identity` | The Ed25519 keypair, its certificate, and `Fingerprint` |
| `tls` | rustls configuration, the identity-pinning verifier, key logging |
| `transport` | `Stream` (plaintext or TLS) and the listener/dialer that produces one |
| `pairing` | The persisted peer store |
| `session` | One connection: the protocol state machine |
| `node` | Every connection: lifecycle, routing, the command surface |
| `discovery` | mDNS advertisement and browsing |
| `clipboard` | The `Clipboard` trait and its implementations |
| `transfer` | Digests, chunked reading, safe writing, filename reduction |
| `control` | The NDJSON control interface |
| `config`, `paths`, `error`, `event`, `ratelimit` | Supporting types |

## Process model

The daemon is one tokio runtime. Work is divided between long-lived tasks that own state
and communicate over channels; nothing shares mutable state except the pairing store.

```
                    ┌──────────────┐
   control clients ─┤  control     │─ one task per client, plus a writer task
                    │  (NDJSON)    │   and a broadcast subscriber task
                    └──────┬───────┘
                    Request│ (command + oneshot reply)
                           ▼
   TCP listener ──►┌──────────────┐◄── Internal (inbound, established, closed)
                   │     Node     │
   mDNS browser ──►│  (one task)  │──► broadcast::Sender<Event>
                   └──────┬───────┘
                 SessionCommand│ (one mpsc per connection)
                           ▼
                   ┌──────────────┐
                   │   Session    │  one task per connection
                   │  (one task)  │──► mpsc::Sender<Event> into Node
                   └──────┬───────┘
                          │ Framed<Stream, NiceCodec>
                          ▼
                        a peer
```

`Node` owns the connection table and is the only thing that assigns a `ConnectionId`.
`Session` owns everything about one connection: its streams, its transfers, its rate
limiter. A `Session` never reaches into `Node` and never touches another session.

Events travel from sessions to `Node` over an mpsc channel, and `Node` republishes them on
a broadcast channel that control clients subscribe to. Routing them through `Node` means
the node sees everything its sessions see, and a slow control client cannot apply
backpressure to a transfer.

Channel capacities: control requests 64, node internals 64, per-session commands 32, the
event mpsc and broadcast 256 each, a control client's outbox 512.

### Why `Node` never awaits a handshake

A command whose answer depends on a handshake — `connect` — hands its reply channel to the
task that performs the handshake. The node loop must not await that answer, because the
answer arrives as an `Internal` message that only the same loop can deliver.

Handshakes therefore always run in a spawned task, subject to `timeouts.handshake_secs`,
and report back as `Internal::Established` or `Internal::Failed`.

## The session loop

Each session runs one `select!` with a fixed priority:

```rust
biased;
command = self.commands.recv()          // instructions from the node
frame   = self.framed.next()            // the peer
_       = ticker.tick()                 // one second
_       = ready(()), if sending         // push another chunk
```

The last branch is a future that is always ready, so it is reached only when the three
above it are pending. An active transfer therefore never starves inbound frames or
control commands, and when there is nothing to send the loop blocks instead of spinning.

Sending is a pump rather than a loop: one chunk per iteration, round-robin over
`send_queue`, so several concurrent transfers interleave and none monopolises the
connection. `framed.send()` awaits its flush, which is where TCP backpressure enters.

## Stream states

A stream exists from the frame that opens it until the frame that ends it. `Session`
holds one state per open stream:

| State | Meaning | Left by |
|---|---|---|
| `OfferedByUs` | `PULL_REQUEST` sent, awaiting a verdict | `MERGE`, `FUCK_OFF`, `BIG_DIFF`, `SHUT_UP` |
| `SendingToPeer` | `MERGE` received, `DIFF` frames in flight | the last chunk, or withdrawal |
| `AwaitingVerification` | `DONE` and `FSCK` sent | `CLEAN` or `CORRUPT` |
| `OfferedToUs` | `PULL_REQUEST` received, awaiting the user | a verdict, or 300 s of silence |
| `ReceivingFromPeer` | `MERGE` sent, writing to a temporary file | `DONE`, or a protocol error |
| `ReceivedAwaitingFsck` | `DONE` received, size matches | `FSCK` |
| `Chat` | An `LKML` conversation | the connection |

Outgoing transfer data lives in a parallel `sends` map rather than inside
`SendingToPeer`, so the send pump can borrow it without borrowing the stream table.

## Error taxonomy

The wire has four ways to say something went wrong, and the code distinguishes them:

| Internal | On the wire | Connection |
|---|---|---|
| `ProtoError` that keeps framing | `CPP` | survives |
| `ProtoError` that loses framing | nothing | closed immediately |
| `Fatal::BrokeUserspace` | `BROKE_USERSPACE` | closed |
| `Fatal::Nvidia` | `NVIDIA` | closed |
| `Fatal::Closed` | nothing | closed |

A frame that could not be parsed is `CPP`. A frame that parsed and is illegal in context
is `BROKE_USERSPACE`. Losing octet alignment — bad magic, an oversized length — means no
reply can be trusted to land where the peer expects it, so the connection closes silently.

An error frame is never answered with an error frame.

## Cross-platform surface

Everything except the control endpoint is portable. `control::platform` supplies a Unix
domain socket on unix and a named pipe on Windows; the NDJSON protocol above it is
identical. The clipboard is behind a trait, and falls back to an in-memory implementation
when no display server is reachable.
