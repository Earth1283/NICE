# nicer

A NICE/1 daemon in Rust.

[`NICE-1.md`](NICE-1.md) is the protocol. [`NICE-1-RFC.md`](NICE-1-RFC.md) resolves what it
leaves open — stream allocation, `DIFF` offset width, frame size negotiation, how Ed25519
reaches the TLS handshake — and is what this implementation is written against. Read that
one before writing a client.

```
crates/nicer-proto   framing, opcodes, CBOR payloads. No state, no policy.
crates/nicer         the daemon: identity, pairing, transport, transfers, chat, discovery
crates/nicer-tui     a ratatui frontend, embedding the daemon directly
```

`nicer` is both the library and the binary.

## Running

```sh
cargo build --release
./target/release/nicer
```

Peers are found over mDNS on `_nice._tcp.local`, port 6969 by default. The daemon holds an
Ed25519 identity in its data directory and speaks TLS 1.3. A frontend drives it over a Unix
socket, or a named pipe on Windows.

```sh
nicer identity          # this device's fingerprint
nicer config            # the effective configuration
nicer run --help
```

## The TUI

```sh
cargo run -p nicer-tui --release
```

`nicer-tui` embeds the daemon library directly (see "Embedding instead" in
[docs/client-guide.md](docs/client-guide.md)) rather than driving it over the socket, so it's
one process, no separate `nicer run` needed. Discovered and paired peers, live connections,
transfers, and chat each get a tab; pairing and incoming offers pop up as modals the moment
they arrive. Press `?` inside it for the keybindings. Pass `--socket PATH` if you also want
another client able to drive the same daemon over NDJSON while the TUI runs.

## Documentation

| | |
|---|---|
| [docs/architecture.md](docs/architecture.md) | Crate and module layout, the task and channel model, the session state machine |
| [docs/behaviour.md](docs/behaviour.md) | What the daemon does, in the order it does it |
| [docs/control-protocol.md](docs/control-protocol.md) | The NDJSON control interface, field by field |
| [docs/client-guide.md](docs/client-guide.md) | Writing a frontend, or embedding the library instead |
| [docs/interop.md](docs/interop.md) | Writing a NICE/1 peer that talks to `nicer` over the network |
| [docs/configuration.md](docs/configuration.md) | `config.toml`, flags, file locations |
| [docs/security.md](docs/security.md) | Identity, pairing, authorisation, resource bounds, threat model |

## The control interface in one screen

Newline-delimited JSON. One object per line each way. Replies echo the command's `id`;
events have no `id`.

```
$ nc -U /run/user/1000/nicer/nicer.sock
{"id":1,"cmd":"connect","address":"192.168.1.42:6969"}
{"id":1,"reply":"connected","connection":1}

{"event":"pairing_required","connection":1,"device":"thinkpad",
 "fingerprint":"9f3a…","short":"9f3a1c04 b7e2d580"}
{"id":2,"cmd":"pair","fingerprint":"9f3a…"}
{"id":2,"reply":"ok"}

{"event":"offer","connection":1,"stream":8,"kind":"file",
 "name":"extremely_important_cat.png","size":812944,"auto_accepted":false}
{"id":3,"cmd":"respond","connection":1,"stream":8,"verdict":"merge"}
{"event":"transfer_complete","connection":1,"stream":8,
 "path":"/home/you/Downloads/nicer/extremely_important_cat.png"}
```

## What the daemon will not do

- Trust a discovered device. Discovery never triggers a connection or a pairing prompt.
- Store an identity without the user saying so.
- Accept an object without explicit approval, unless the user marked that peer
  auto-accepting for that kind. Pairing alone is not permission.
- Write a received file anywhere but the download directory, under a name the sender chose,
  or over a file that already exists.
- Leave a partial or unverified object where it could be mistaken for a finished one.
- Speak PLAINTEXT because a flag asked it to. The configuration file must also say
  `transport.i_know_wireshark_can_read_this = true`.

## Tests

```sh
cargo test
```

`nicer-proto/tests/wire.rs` covers the frame format, `nicer/tests/handshake.rs` the Ed25519
identity binding, `nicer/tests/control_contract.rs` the NDJSON surface,
`nicer/tests/end_to_end.rs` two daemons against each other, and `nicer/tests/hardening.rs` a
hand-rolled peer misbehaving at one.

## Status

The protocol surface is complete: both transport modes, discovery, pairing, offers, file
and clipboard transfer, integrity checking, chat, rate limiting, recovery, and the error
opcodes. Windows is written for but untested.

`nicer-tui` is a working frontend; a socket-based one in another language is still open.
